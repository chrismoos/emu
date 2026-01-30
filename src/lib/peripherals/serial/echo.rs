use std::sync::Mutex;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::peripherals::serial::{SerialDevice, SerialDeviceConnection};

pub struct EchoSerialPort {}

impl EchoSerialPort {
    pub fn new() -> EchoSerialPort {
        EchoSerialPort {}
    }
}

impl SerialDevice for EchoSerialPort {
    fn open<'a>(
        &'a self,
        _options: super::SerialDeviceOptions,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        std::pin::Pin<Box<dyn super::SerialDeviceConnection>>,
                        crate::errors::Error,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async {
            let conn: Box<dyn SerialDeviceConnection> = Box::new(Connection::new());
            Ok(Box::into_pin(conn))
        })
    }
}

struct Connection {
    buf: Mutex<Vec<u8>>,
}

impl Connection {
    pub fn new() -> Connection {
        Connection {
            buf: Mutex::new(vec![]),
        }
    }
}

impl SerialDeviceConnection for Connection {}

impl AsyncWrite for Connection {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let mut b = self.buf.lock().unwrap();
        b.extend(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
}

impl AsyncRead for Connection {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let mut b = self.buf.lock().unwrap();
        if b.len() == 0 {
            return std::task::Poll::Pending;
        }
        let n = b.len().min(buf.remaining());
        buf.put_slice(&b[0..n]);
        b.drain(0..n);
        std::task::Poll::Ready(Ok(()))
    }
}
