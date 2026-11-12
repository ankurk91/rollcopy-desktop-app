use idevice::IdeviceError;

/// Everything that can go wrong talking to a phone, already sorted into the
/// states the connection guide (PLAN §5) shows. Each variant maps to exactly
/// one "next step" for the user.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    /// The usbmuxd socket isn't there or refuses connections. It only exists
    /// while an Apple device is plugged in (udev starts the daemon on demand),
    /// so this alone does not mean usbmuxd is missing.
    #[error("usbmuxd is not reachable ({0})")]
    UsbmuxdUnavailable(String),

    /// The requested device is not (or no longer) listed by usbmuxd.
    #[error("device is not connected")]
    NotConnected,

    /// No pair record for this phone on this computer. usbmuxd 1.1+ starts
    /// pairing by itself when such a phone is plugged in.
    #[error("this computer is not trusted by the device")]
    NotPaired,

    /// A pair record exists but the phone rejects it (`InvalidHostID`, e.g.
    /// after an iOS update or a privacy reset). usbmuxd won't re-pair on its
    /// own in this case.
    #[error("the device no longer trusts this computer")]
    StalePairing,

    /// A Trust dialog is showing on the phone and hasn't been answered.
    #[error("waiting for Trust to be tapped on the device")]
    PairingPending,

    /// The user tapped Don't Trust. iOS won't ask again until re-plugged.
    #[error("the device said Don't Trust; unplug and re-plug to ask again")]
    PairingDenied,

    /// The phone is locked (or has never been unlocked since boot).
    #[error("the device is locked")]
    Locked,

    /// A path on the device does not exist.
    #[error("not found on device: {0}")]
    NotFound(String),

    /// The connection dropped mid-operation (cable pulled, phone locked during
    /// AFC). A transfer queue should pause, not fail, on this.
    #[error("connection to the device was lost ({0})")]
    Disconnected(String),

    /// A local file error while saving something read from the device.
    #[error("local I/O error: {0}")]
    LocalIo(#[from] std::io::Error),

    #[error("device error: {0}")]
    Other(String),
}

impl DeviceError {
    /// True when retrying after the user fixes something (unlock, re-plug,
    /// tap Trust) is the right response, rather than recording a failure.
    pub fn is_connection_problem(&self) -> bool {
        matches!(
            self,
            Self::UsbmuxdUnavailable(_)
                | Self::NotConnected
                | Self::NotPaired
                | Self::StalePairing
                | Self::PairingPending
                | Self::PairingDenied
                | Self::Locked
                | Self::Disconnected(_)
        )
    }
}

impl From<IdeviceError> for DeviceError {
    fn from(e: IdeviceError) -> Self {
        use idevice::afc::errors::AfcError;
        match e {
            IdeviceError::InvalidHostID => Self::StalePairing,
            IdeviceError::PairingDialogResponsePending => Self::PairingPending,
            IdeviceError::UserDeniedPairing => Self::PairingDenied,
            IdeviceError::PasswordProtected | IdeviceError::DeviceLocked => Self::Locked,
            IdeviceError::DeviceNotFound => Self::NotConnected,
            IdeviceError::Socket(io) => Self::Disconnected(io.to_string()),
            IdeviceError::Afc(AfcError::ObjectNotFound) => Self::NotFound(String::new()),
            // Seen 2026-10-19 when the phone auto-locked mid-transfer.
            IdeviceError::Afc(
                a @ (AfcError::IoError | AfcError::MuxError | AfcError::ServiceNotConnected),
            ) => Self::Disconnected(a.to_string()),
            other => Self::Other(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, DeviceError>;
