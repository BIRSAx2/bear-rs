/// Post the Darwin notification that tells Bear to refresh its UI after a
/// local database write.
///
/// Uses `notifyutil -p` (ships with macOS). If the tool is absent or fails
/// the write still lands; Bear picks it up on the next file-watch tick.
pub fn request_app_refresh() {
    let _ = std::process::Command::new("notifyutil")
        .arg("-p")
        .arg("net.shinyfrog.bear.cli.didRequestRefresh")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}
