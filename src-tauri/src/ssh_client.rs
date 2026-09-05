use anyhow::{Context, Result};
use ssh2::Session;
use std::io::Read;
use std::net::TcpStream;
use std::path::Path;

pub trait CommandRunner: Send {
    fn run(&mut self, command: &str) -> Result<String>;
}

pub struct SshClient {
    session: Session,
}

impl SshClient {
    pub fn connect(
        host: &str,
        port: u16,
        username: &str,
        key_path: &Path,
        passphrase: Option<&str>,
    ) -> Result<Self> {
        let tcp = TcpStream::connect((host, port))
            .with_context(|| format!("connecting to {host}:{port}"))?;
        let mut session = Session::new().context("creating ssh session")?;
        session.set_tcp_stream(tcp);
        session.set_timeout(30_000); // 30s: fail fast on a hung/half-open connection, including handshake
        session.handshake().context("ssh handshake failed")?;
        session
            .userauth_pubkey_file(username, None, key_path, passphrase)
            .with_context(|| format!("authenticating as {username} with key {key_path:?}"))?;
        if !session.authenticated() {
            anyhow::bail!("ssh authentication failed for user {username}");
        }
        Ok(Self { session })
    }
}

impl CommandRunner for SshClient {
    fn run(&mut self, command: &str) -> Result<String> {
        let mut channel = self
            .session
            .channel_session()
            .context("opening ssh channel")?;
        channel.exec(command).with_context(|| format!("executing `{command}`"))?;
        let mut output = String::new();
        channel
            .read_to_string(&mut output)
            .context("reading command output")?;
        let mut stderr_output = String::new();
        channel
            .stderr()
            .read_to_string(&mut stderr_output)
            .context("reading command stderr")?;
        channel.wait_close().context("closing ssh channel")?;
        let exit_status = channel.exit_status().context("reading exit status")?;
        if exit_status != 0 {
            anyhow::bail!(
                "command `{command}` exited with status {exit_status}: stdout={output:?} stderr={stderr_output:?}"
            );
        }
        Ok(output)
    }
}
