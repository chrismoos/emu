use std::pin::Pin;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::errors::Error;

pub mod echo;
#[cfg(feature = "native")]
pub mod internet_modem;
#[cfg(feature = "native")]
pub mod port;

#[derive(Debug, PartialEq, Eq)]
pub enum SerialParity {
    None,
    Odd,
    Even,
    Mark,
    Space,
}

pub struct SerialDeviceOptions {
    pub parity: SerialParity,
    pub baud: usize,
}

pub trait SerialDevice: Send + Sync {
    fn open<'a>(
        &'a self,
        options: SerialDeviceOptions,
    ) -> Pin<
        Box<dyn Future<Output = Result<Pin<Box<dyn SerialDeviceConnection>>, Error>> + Send + 'a>,
    >;
}

pub trait SerialDeviceConnection: AsyncRead + AsyncWrite + Send + Sync {}
