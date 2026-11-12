use idevice::IdeviceService;
use idevice::afc::AfcClient;
use idevice::lockdown::LockdownClient;
use idevice::provider::{IdeviceProvider, UsbmuxdProvider};
use idevice::usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection};
use plist::{Dictionary, Value};

use crate::error::{DeviceError, Result};
use crate::media::MediaFs;
use crate::models;

const LABEL: &str = "unspool";

/// A phone as listed by usbmuxd, before any lockdown handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRef {
    pub udid: String,
    /// usbmuxd's id for this attachment. Changes on every re-plug.
    pub mux_id: u32,
    pub usb: bool,
}

/// Device facts shown on the device card (PLAN §3). Optional fields are
/// queries the phone may refuse; their absence is not an error.
#[derive(Debug, Clone, Default)]
pub struct DeviceInfo {
    pub udid: String,
    pub name: String,
    /// Raw identifier, e.g. `iPhone15,4`.
    pub product_type: String,
    /// Marketing name, e.g. `iPhone 15`, or the raw identifier if unknown.
    pub model: String,
    pub ios_version: String,
    pub build_version: String,
    pub serial_number: Option<String>,
    /// Marketing capacity in bytes (128 000 000 000 for "128 GB").
    pub storage_total: Option<u64>,
    pub storage_free: Option<u64>,
    pub battery_percent: Option<u8>,
    pub battery_charging: Option<bool>,
    pub icloud_backup: Option<bool>,
}

fn usbmuxd_unavailable(e: idevice::IdeviceError) -> DeviceError {
    DeviceError::UsbmuxdUnavailable(e.to_string())
}

/// Lists phones usbmuxd currently sees. A missing socket is reported as
/// [`DeviceError::UsbmuxdUnavailable`], which in practice usually means "no
/// Apple device plugged in" (the daemon is started on demand by udev).
pub async fn list_devices() -> Result<Vec<DeviceRef>> {
    let mut mux = UsbmuxdConnection::default()
        .await
        .map_err(usbmuxd_unavailable)?;
    let devices = mux.get_devices().await.map_err(usbmuxd_unavailable)?;
    Ok(devices
        .into_iter()
        .map(|d| DeviceRef {
            usb: d.connection_type == Connection::Usb,
            udid: d.udid,
            mux_id: d.device_id,
        })
        .collect())
}

/// A trusted, unlocked connection to one phone.
///
/// Opening a session proves the pairing works: it fails with
/// [`DeviceError::NotPaired`] / [`DeviceError::Locked`] / ... so the caller
/// can show the right connection-guide screen.
///
/// Read-only by construction (PLAN §2a): the lockdown client is private and
/// only `GetValue` is ever sent on it; file access goes through [`MediaFs`].
pub struct Session {
    provider: UsbmuxdProvider,
    lockdown: LockdownClient,
    udid: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("udid", &self.udid).finish()
    }
}

impl Session {
    /// Connects to the phone with this UDID and starts a lockdown session.
    pub async fn open(udid: &str) -> Result<Self> {
        let mut mux = UsbmuxdConnection::default()
            .await
            .map_err(usbmuxd_unavailable)?;
        let device = match mux.get_device(udid).await {
            Ok(d) => d,
            Err(idevice::IdeviceError::DeviceNotFound) => return Err(DeviceError::NotConnected),
            Err(e) => return Err(e.into()),
        };
        // usbmuxd answers with an error when it has no record for this phone.
        let pairing = mux
            .get_pair_record(udid)
            .await
            .map_err(|_| DeviceError::NotPaired)?;

        let provider = device.to_provider(UsbmuxdAddr::default(), LABEL);
        let mut lockdown = LockdownClient::connect(&provider).await?;
        lockdown.start_session(&pairing).await?;

        Ok(Self {
            provider,
            lockdown,
            udid: udid.to_owned(),
        })
    }

    pub fn udid(&self) -> &str {
        &self.udid
    }

    async fn get(&mut self, key: Option<&str>, domain: Option<&str>) -> Result<Value> {
        Ok(self.lockdown.get_value(key, domain).await?)
    }

    async fn domain(&mut self, domain: &str) -> Option<Dictionary> {
        self.get(None, Some(domain))
            .await
            .ok()
            .and_then(Value::into_dictionary)
    }

    /// Reads the device card facts. Required keys failing is an error;
    /// optional domains (storage, battery, backup) are best-effort.
    pub async fn info(&mut self) -> Result<DeviceInfo> {
        let root = self
            .get(None, None)
            .await?
            .into_dictionary()
            .ok_or_else(|| DeviceError::Other("lockdown root is not a dictionary".into()))?;
        let s = |k: &str| root.get(k).and_then(Value::as_string).map(str::to_owned);

        let product_type = s("ProductType").unwrap_or_default();
        let mut info = DeviceInfo {
            udid: s("UniqueDeviceID").unwrap_or_else(|| self.udid.clone()),
            name: s("DeviceName").unwrap_or_default(),
            model: models::display_model(&product_type),
            product_type,
            ios_version: s("ProductVersion").unwrap_or_default(),
            build_version: s("BuildVersion").unwrap_or_default(),
            serial_number: s("SerialNumber"),
            ..Default::default()
        };

        if let Some(d) = self.domain("com.apple.disk_usage").await {
            info.storage_total = uint(&d, "TotalDiskCapacity");
            info.storage_free = uint(&d, "TotalDataAvailable");
        }
        if let Some(d) = self.domain("com.apple.mobile.battery").await {
            info.battery_percent = uint(&d, "BatteryCurrentCapacity").map(|v| v.min(100) as u8);
            info.battery_charging = d.get("BatteryIsCharging").and_then(Value::as_boolean);
        }
        if let Some(d) = self.domain("com.apple.mobile.backup").await {
            info.icloud_backup = d.get("CloudBackupEnabled").and_then(Value::as_boolean);
        }
        Ok(info)
    }

    /// Opens the media filesystem (`/var/mobile/Media`: `DCIM/`, `PhotoData/`)
    /// over AFC, read-only. Each call opens an independent connection, so two
    /// can read in parallel.
    pub async fn media(&self) -> Result<MediaFs> {
        let afc = AfcClient::connect(&self.provider as &dyn IdeviceProvider).await?;
        Ok(MediaFs::new(afc))
    }
}

fn uint(d: &Dictionary, key: &str) -> Option<u64> {
    d.get(key).and_then(|v| {
        v.as_unsigned_integer()
            .or_else(|| v.as_signed_integer().and_then(|i| u64::try_from(i).ok()))
    })
}
