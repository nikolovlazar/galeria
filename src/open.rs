//! Hand things to the system: links to the browser, folders and files to the file manager.

use std::path::Path;
use std::process::Command;

fn run(mut cmd: Command) {
    match cmd.spawn() {
        Ok(mut child) => {
            // Reap it so it does not linger as a zombie.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::warn!("could not open: {e}"),
    }
}

/// Open a web link in the default browser. Anything that is not http(s) is refused.
pub fn url(url: &str) -> bool {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        log::warn!("refusing to open {url}: only web links can be opened");
        return false;
    }
    open_arg(url);
    true
}

/// Open a folder in the file manager, or a file in its default app.
pub fn path(path: &Path) {
    open_arg(&path.to_string_lossy());
}

fn open_arg(arg: &str) {
    #[cfg(target_os = "macos")]
    let cmd = {
        let mut c = Command::new("open");
        c.arg(arg);
        c
    };
    #[cfg(target_os = "windows")]
    let cmd = {
        // `explorer` opens folders, files and links alike, and takes them as one argument.
        let mut c = Command::new("explorer");
        c.arg(arg);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(arg);
        c
    };
    run(cmd);
}

/// Show a file in the file manager, selected.
pub fn reveal(file: &Path) {
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        c.arg("-R").arg(file);
        run(c);
    }
    #[cfg(target_os = "windows")]
    {
        let mut c = Command::new("explorer");
        c.arg(format!("/select,{}", file.display()));
        run(c);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // No portable "select this file": open the folder that holds it.
        path(file.parent().unwrap_or(file));
    }
}
