# FAQ

Questions people ask about [Rollcopy](../README.md). For problems, see
[Troubleshooting](TROUBLESHOOTING.md).

**How do I transfer photos from an iPhone to Ubuntu or Linux Mint?**
Install Rollcopy, connect the iPhone with a USB cable, tap **Trust** on the phone, select the photos
and click **Copy selected**. No iTunes, iCloud or Apple account is needed.

**Can it convert HEIC to JPG?**
Yes, while copying. HEIC photos become JPG (quality 92 by default, colour profile and EXIF kept) and
MOV videos become MP4. JPG, PNG and MP4 files are copied unchanged.

**Does it copy Live Photos and edited photos?**
Edited photos and videos are copied as edited. Live Photos are copied as the still photo; turn on
*Preferences → Conversion → Live Photos: also copy the motion* to get the moving part as a video too.

**Does it work on Fedora, Arch or other distributions?**
The `.deb` is for Ubuntu 24.04+ and its derivatives. The AppImage may run on other 64-bit distributions
with glibc 2.39 or newer, with `usbmuxd` and `ffmpeg` installed; this isn't tested yet.

**Can it delete photos from the iPhone or free up space?**
No. Rollcopy only reads from the phone; it never changes or deletes anything on it.
