use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostSignal {
    Interrupt,
    #[cfg(unix)]
    Terminate,
}

impl HostSignal {
    pub(crate) const fn number(self) -> i32 {
        match self {
            Self::Interrupt => 2,
            #[cfg(unix)]
            Self::Terminate => 15,
        }
    }

    pub(crate) const fn exit_status(self) -> u8 {
        match self {
            Self::Interrupt => 130,
            #[cfg(unix)]
            Self::Terminate => 143,
        }
    }
}

#[cfg(unix)]
pub(crate) struct HostSignals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl HostSignals {
    pub(crate) fn new() -> io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};

        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }

    pub(crate) async fn receive(&mut self) -> io::Result<HostSignal> {
        tokio::select! {
            received = self.interrupt.recv() => received.map_or_else(
                || Err(io::Error::other("interrupt signal listener closed")),
                |()| Ok(HostSignal::Interrupt),
            ),
            received = self.terminate.recv() => received.map_or_else(
                || Err(io::Error::other("termination signal listener closed")),
                |()| Ok(HostSignal::Terminate),
            ),
        }
    }
}

#[cfg(windows)]
pub(crate) struct HostSignals {
    interrupt: tokio::signal::windows::CtrlC,
}

#[cfg(windows)]
impl HostSignals {
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self { interrupt: tokio::signal::windows::ctrl_c()? })
    }

    pub(crate) async fn receive(&mut self) -> io::Result<HostSignal> {
        self.interrupt.recv().await.map_or_else(
            || Err(io::Error::other("interrupt signal listener closed")),
            |()| Ok(HostSignal::Interrupt),
        )
    }
}
