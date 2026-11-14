//! One running Galeria per user. A second launch hands its command line to the
//! first and exits, which is how `galeria --next` controls the running app.
//!
//! On Unix the first instance listens on a socket in its data folder. Windows
//! has no socket files, so it listens on the loopback interface and leaves the
//! port, with a random token only this user can read, beside its data.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The first instance's side: waits for later launches.
pub struct Listener {
    inner: imp::Listener,
    token: String,
    cleanup: PathBuf,
}

pub enum Claim {
    /// Nobody else is running. Call [`Listener::serve`] once the app is up.
    First(Listener),
    /// Another instance took this launch's arguments; exit.
    Forwarded,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Message {
    token: String,
    args: Vec<String>,
}

/// Become the running instance, or hand `args` to the one that already is.
/// `dir` is the data folder; with a sandboxed `GALERIA_HOME` it differs, so a
/// test run and the installed app don't talk to each other.
pub fn claim(dir: &Path, args: &[String]) -> Claim {
    if let Some(mut stream) = imp::connect(dir) {
        let msg = Message {
            token: imp::token(dir),
            args: args.to_vec(),
        };
        let sent = serde_json::to_string(&msg)
            .ok()
            .and_then(|line| writeln!(stream, "{line}").ok())
            .is_some();
        if sent {
            let _ = stream.flush();
            let mut reply = String::new();
            let _ = BufReader::new(stream).read_line(&mut reply);
            if reply.trim() == "ok" {
                return Claim::Forwarded;
            }
        }
        // Something answered but it was not Galeria (or it is shutting down): start normally.
    }
    match imp::listen(dir) {
        Ok((inner, token, cleanup)) => Claim::First(Listener {
            inner,
            token,
            cleanup,
        }),
        Err(e) => {
            log::warn!("single-instance control is unavailable: {e}");
            Claim::First(Listener {
                inner: imp::Listener::none(),
                token: String::new(),
                cleanup: PathBuf::new(),
            })
        }
    }
}

impl Listener {
    /// Call `on_launch` with the arguments of every later launch, on a thread of its own.
    pub fn serve(self, on_launch: impl Fn(Vec<String>) + Send + 'static) {
        let Listener {
            inner,
            token,
            cleanup: _,
        } = self;
        let spawned = std::thread::Builder::new()
            .name("galeria-instance".into())
            .spawn(move || {
                inner.accept_each(|stream| {
                    let _ = stream.set_timeouts(Duration::from_secs(5));
                    let mut line = String::new();
                    let Ok(mut reader) = stream.reader() else {
                        return;
                    };
                    if reader.read_line(&mut line).is_err() {
                        return;
                    }
                    let Ok(msg) = serde_json::from_str::<Message>(&line) else {
                        return;
                    };
                    if msg.token != token {
                        return;
                    }
                    let _ = stream.reply("ok\n");
                    on_launch(msg.args);
                });
            });
        if let Err(e) = spawned {
            log::warn!("single-instance control is unavailable: {e}");
        }
    }

    /// Where the socket (or port file) lives, to remove on exit.
    pub fn endpoint(&self) -> &Path {
        &self.cleanup
    }
}

/// Remove what [`claim`] left on disk. Call on the way out.
pub fn release(endpoint: &Path) {
    if !endpoint.as_os_str().is_empty() {
        let _ = std::fs::remove_file(endpoint);
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::net::{UnixListener, UnixStream};

    /// Socket paths are limited to about 100 bytes; a deep data folder falls back to the temp dir.
    fn socket_path(dir: &Path) -> PathBuf {
        let path = dir.join("galeria.sock");
        if path.as_os_str().len() < 100 {
            return path;
        }
        std::env::temp_dir().join(format!(
            "galeria-{}.sock",
            &crate::util::hash_str(&[&dir.to_string_lossy()])[..16]
        ))
    }

    /// The socket file is only reachable by this user, so no secret is needed.
    pub fn token(_dir: &Path) -> String {
        String::new()
    }

    pub fn connect(dir: &Path) -> Option<Stream> {
        let s = UnixStream::connect(socket_path(dir)).ok()?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
        Some(Stream(s))
    }

    pub fn listen(dir: &Path) -> std::io::Result<(Listener, String, PathBuf)> {
        let path = socket_path(dir);
        // `connect` just failed, so a file here is left over from a crash.
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        #[allow(clippy::permissions_set_readonly_false)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok((Listener(Some(listener)), String::new(), path))
    }

    pub struct Listener(Option<UnixListener>);

    impl Listener {
        pub fn none() -> Self {
            Listener(None)
        }

        pub fn accept_each(self, mut each: impl FnMut(Stream)) {
            let Some(listener) = self.0 else { return };
            for stream in listener.incoming().flatten() {
                each(Stream(stream));
            }
        }
    }

    pub struct Stream(UnixStream);

    impl Stream {
        pub fn set_timeouts(&self, t: Duration) -> std::io::Result<()> {
            self.0.set_read_timeout(Some(t))?;
            self.0.set_write_timeout(Some(t))
        }

        pub fn reader(&self) -> std::io::Result<BufReader<UnixStream>> {
            Ok(BufReader::new(self.0.try_clone()?))
        }

        pub fn reply(&self, text: &str) -> std::io::Result<()> {
            (&self.0).write_all(text.as_bytes())
        }
    }

    impl std::io::Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};

    fn port_file(dir: &Path) -> PathBuf {
        dir.join("instance.port")
    }

    /// `port token`, as the running instance wrote it.
    fn read(dir: &Path) -> Option<(u16, String)> {
        let text = std::fs::read_to_string(port_file(dir)).ok()?;
        let (port, token) = text.trim().split_once(' ')?;
        Some((port.parse().ok()?, token.to_string()))
    }

    pub fn token(dir: &Path) -> String {
        read(dir).map(|(_, t)| t).unwrap_or_default()
    }

    pub fn connect(dir: &Path) -> Option<Stream> {
        let (port, _) = read(dir)?;
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let s = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
        Some(Stream(s))
    }

    pub fn listen(dir: &Path) -> std::io::Result<(Listener, String, PathBuf)> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let token = format!("{:016x}{:016x}", fastrand::u64(..), fastrand::u64(..));
        let path = port_file(dir);
        std::fs::write(&path, format!("{port} {token}"))?;
        Ok((Listener(Some(listener)), token, path))
    }

    pub struct Listener(Option<TcpListener>);

    impl Listener {
        pub fn none() -> Self {
            Listener(None)
        }

        pub fn accept_each(self, mut each: impl FnMut(Stream)) {
            let Some(listener) = self.0 else { return };
            for stream in listener.incoming().flatten() {
                each(Stream(stream));
            }
        }
    }

    pub struct Stream(TcpStream);

    impl Stream {
        pub fn set_timeouts(&self, t: Duration) -> std::io::Result<()> {
            self.0.set_read_timeout(Some(t))?;
            self.0.set_write_timeout(Some(t))
        }

        pub fn reader(&self) -> std::io::Result<BufReader<TcpStream>> {
            Ok(BufReader::new(self.0.try_clone()?))
        }

        pub fn reply(&self, text: &str) -> std::io::Result<()> {
            (&self.0).write_all(text.as_bytes())
        }
    }

    impl std::io::Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_second_launch_reaches_the_first() {
        let dir = std::env::temp_dir().join(format!("galeria-inst-{}", fastrand::u64(..)));
        std::fs::create_dir_all(&dir).unwrap();
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        let Claim::First(listener) = claim(&dir, &args(&["galeria"])) else {
            panic!("nothing was running, so this launch is the first");
        };
        let endpoint = listener.endpoint().to_path_buf();
        let (tx, rx) = mpsc::channel();
        listener.serve(move |a| {
            let _ = tx.send(a);
        });

        assert!(matches!(
            claim(&dir, &args(&["galeria", "--next"])),
            Claim::Forwarded
        ));
        let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(got, args(&["galeria", "--next"]));

        release(&endpoint);
        std::fs::remove_dir_all(&dir).ok();
    }
}
