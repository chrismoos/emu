use std::{
    ops::DerefMut,
    pin::Pin,
    sync::Mutex,
};

use log::{debug, error};
use serialport::Parity;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_serial::SerialStream;

use crate::peripherals::serial::{SerialDevice, SerialDeviceConnection};

pub struct SerialDevicePort {
    path: String,
    force_zero_baud: bool,
}

impl SerialDevicePort {
    pub fn new(path: &str, force_zero_baud: bool) -> SerialDevicePort {
        SerialDevicePort {
            path: path.to_owned(),
            force_zero_baud,
        }
    }
}

struct SerialDevicePortConnection {
    stream: Mutex<SerialStream>,
}

impl SerialDeviceConnection for SerialDevicePortConnection {}

impl SerialDevice for SerialDevicePort {
    fn open<'a>(
        &'a self,
        options: super::SerialDeviceOptions,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Pin<Box<dyn SerialDeviceConnection>>, crate::errors::Error>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            debug!(
                "Opening serial port {}, baud {}, parity: {:?}",
                self.path, options.baud, options.parity
            );
            let conn: Box<dyn SerialDeviceConnection> = Box::new(SerialDevicePortConnection {
                stream: Mutex::new(SerialStream::open(&serialport::new(
                    &self.path,
                    if self.force_zero_baud {
                        0
                    } else {
                        options.baud as u32
                    },
                ).parity(match options.parity {
                    crate::peripherals::serial::SerialParity::None => Parity::None,
                    crate::peripherals::serial::SerialParity::Odd => Parity::Odd,
                    crate::peripherals::serial::SerialParity::Even => Parity::Even,
                    _ => {
                        error!("unsupported parity option: {:?}, falling back to none", options.parity);
                        Parity::None
                    },
                }))?),
            });
            Ok(Box::into_pin(conn))
        })
    }
}

impl AsyncRead for SerialDevicePortConnection {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let mut opt = self.stream.lock().unwrap();
        let stream = opt.deref_mut();
        let pinned = std::pin::pin!(stream);
        pinned.poll_read(cx, buf)
    }
}

impl AsyncWrite for SerialDevicePortConnection {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let mut opt = self.stream.lock().unwrap();
        let stream = opt.deref_mut();
        let pinned = std::pin::pin!(stream);
        pinned.poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let mut opt = self.stream.lock().unwrap();
        let stream = opt.deref_mut();
        let pinned = std::pin::pin!(stream);
        pinned.poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let mut opt = self.stream.lock().unwrap();
        let stream = opt.deref_mut();
        let pinned = std::pin::pin!(stream);
        pinned.poll_shutdown(cx)
    }
}
