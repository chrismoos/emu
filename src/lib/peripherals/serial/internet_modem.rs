use std::{
    pin::{Pin, pin},
    sync::Mutex,
    time::Duration,
};

use log::{debug, error, trace};
use tokio::{
    io::{
        AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream, duplex,
    },
    time::timeout,
};

use crate::{
    errors::Error,
    peripherals::serial::{SerialDevice, SerialDeviceConnection},
    utils::futures::spawn,
};

pub struct InternetModem {}

impl InternetModem {
    pub fn new() -> InternetModem {
        InternetModem {}
    }
}

struct InternetModemConnection {
    client: Mutex<DuplexStream>,
}

impl SerialDeviceConnection for InternetModemConnection {}

impl InternetModemConnection {
    async fn read_command<R, W>(reader: R, writer: W) -> Result<String, Error>
    where
        R: AsyncRead,
        W: AsyncWrite,
    {
        let mut buf = Vec::new();
        let mut reader = pin!(reader);
        let mut writer = pin!(writer);
        loop {
            let byte = reader.read_u8().await?;
            if byte == b'\n' || byte == b'\r' {
                if buf.len() > 0 {
                    writer.write_all(b"\r\n").await?;
                }
                break;
            }
            writer.write_all(&[byte]).await?;
            buf.push(byte);
        }
        Ok(String::from_utf8(buf).unwrap_or_default().trim().to_owned())
    }

    pub fn new() -> InternetModemConnection {
        let (client, server) = duplex(4096);
        spawn(async move {
            if let Err(e) = Self::run(server).await {
                error!("error in modem handler: {:?}", e);
            }
        });
        InternetModemConnection {
            client: Mutex::new(client),
        }
    }

    async fn send_response<T: AsyncWrite + Unpin>(mut tx: T, line: &str) -> Result<(), Error> {
        tx.write_all(line.as_bytes()).await?;
        tx.flush().await?;
        Ok(())
    }

    async fn run(server: DuplexStream) -> Result<(), Error> {
        let (rx, mut tx) = tokio::io::split(server);
        let mut reader = BufReader::new(rx);
        loop {
            let line = Self::read_command(&mut reader, &mut tx)
                .await?
                .to_lowercase();

            if let Some(s) = line.strip_prefix("atdt") {
                let dial = s.trim();
                trace!("Dial {}", dial);
                Self::send_response(&mut tx, &format!("Connecting to {}...\r\n", dial)).await?;

                let mut buf = vec![0u8; 2048];
                let mut server_buf = vec![0u8; 2048];

                match timeout(Duration::from_secs(5), tokio::net::TcpStream::connect(dial)).await {
                    Ok(Ok(mut stream)) => {
                        trace!("connected to {}", dial);
                        Self::send_response(&mut tx, &format!("OK: Connected to {}\r\n", dial))
                            .await?;
                        loop {
                            tokio::select! {
                                client_rx = (stream.read(&mut buf)) => {
                                    match client_rx {
                                        Ok(n) => {
                                            if n == 0 {
                                                trace!("connection EOF reached, closing port");
                                                return Ok(());
                                            }
                                            tx.write_all(&buf[0..n]).await?;
                                            tx.flush().await?;
                                        },
                                        Err(e) => {
                                            trace!("connection error: {:?}", e);
                                            return Err(e.into());
                                        }
                                    }
                                },
                                server_rx = (reader.read(&mut server_buf)) => {
                                    match server_rx {
                                        Ok(n) => {
                                            if n == 0 {
                                                trace!("server connection EOF reached");
                                                return Ok(());
                                            }
                                            stream.write_all(&server_buf[0..n]).await?;
                                            stream.flush().await?;
                                        },
                                        Err(e) => {
                                            trace!("server connection error: {:?}", e);
                                            return Err(e.into());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => {
                        Self::send_response(&mut tx, "ERR: Connect timed out.\r\n").await?;
                    }
                    Ok(Err(e)) => {
                        Self::send_response(
                            &mut tx,
                            &format!("ERR: Failed to connect: {:?}\r\n", e),
                        )
                        .await?;
                    }
                }
            } else {
                Self::send_response(&mut tx, "ERR: unknown command\r\n").await?;
            }
        }
    }
}

impl SerialDevice for InternetModem {
    fn open<'a>(
        &'a self,
        _options: super::SerialDeviceOptions,
    ) -> std::pin::Pin<
        Box<
            dyn Future<Output = Result<Pin<Box<dyn SerialDeviceConnection>>, crate::errors::Error>>
                + Send
                + 'a,
        >,
    > {
        let conn: Box<dyn SerialDeviceConnection> = Box::new(InternetModemConnection::new());
        Box::pin(async move {
            debug!("Modem open port");
            Ok(Box::into_pin(conn))
        })
    }
}

impl AsyncRead for InternetModemConnection {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let client = &mut *self.client.lock().unwrap();
        pin!(client).poll_read(cx, buf)
    }
}

impl AsyncWrite for InternetModemConnection {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let client = &mut *self.client.lock().unwrap();
        pin!(client).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let client = &mut *self.client.lock().unwrap();
        pin!(client).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let client = &mut *self.client.lock().unwrap();
        pin!(client).poll_shutdown(cx)
    }
}
