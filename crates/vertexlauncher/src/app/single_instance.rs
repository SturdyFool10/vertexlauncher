use vertex_i18n::tr;
use launcher_runtime as tokio_runtime;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use vertex_constants::launcher::single_instance::{
    HELLO_MESSAGE, PORT as SINGLE_INSTANCE_PORT, PRESENT_MESSAGE, PROBE_ATTEMPTS, PROBE_TIMEOUT,
    RETRY_DELAY,
};

#[path = "single_instance/single_instance_error.rs"]
mod single_instance_error;
#[path = "single_instance/single_instance_guard.rs"]
mod single_instance_guard;

pub use self::single_instance_error::SingleInstanceError;
pub use self::single_instance_guard::SingleInstanceGuard;

pub fn acquire_single_instance() -> Result<SingleInstanceGuard, SingleInstanceError> {
    let endpoint = SocketAddrV4::new(Ipv4Addr::LOCALHOST, SINGLE_INSTANCE_PORT);
    match UdpSocket::bind(endpoint) {
        Ok(socket) => start_responder(socket, endpoint),
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
            if existing_instance_responded(endpoint)? {
                return Err(SingleInstanceError::AlreadyRunning);
            }
            std::thread::sleep(RETRY_DELAY);
            let socket = UdpSocket::bind(endpoint).map_err(|retry_err| {
                SingleInstanceError::Unavailable(tr!("single-instance IPC endpoint {endpoint} remained unavailable after probe: {retry_err}", endpoint = endpoint, retry_err = retry_err).into_owned())
            })?;
            start_responder(socket, endpoint)
        }
        Err(err) => Err(SingleInstanceError::Unavailable(tr!("failed to bind single-instance IPC endpoint {endpoint}: {err}", endpoint = endpoint, err = err).into_owned())),
    }
}

fn start_responder(
    socket: UdpSocket,
    endpoint: SocketAddrV4,
) -> Result<SingleInstanceGuard, SingleInstanceError> {
    let stop_requested = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop_requested);
    let (completion_tx, completion_rx) = mpsc::channel::<()>();
    let _ = tokio_runtime::spawn_blocking_detached(move || {
        run_responder(socket, worker_stop);
        if let Err(err) = completion_tx.send(()) {
            tracing::error!(
                target: "vertexlauncher/single_instance",
                error = %err,
                "Failed to deliver single-instance responder completion signal."
            );
        }
    });
    Ok(SingleInstanceGuard {
        endpoint,
        stop_requested,
        completion_rx: Some(completion_rx),
    })
}

fn run_responder(socket: UdpSocket, stop_requested: Arc<AtomicBool>) {
    let mut buffer = [0_u8; 128];
    while !stop_requested.load(Ordering::SeqCst) {
        let Ok((len, source)) = socket.recv_from(&mut buffer) else {
            if stop_requested.load(Ordering::SeqCst) {
                break;
            }
            continue;
        };
        if stop_requested.load(Ordering::SeqCst) {
            break;
        }
        if &buffer[..len] == HELLO_MESSAGE {
            let _ = socket.send_to(PRESENT_MESSAGE, source);
        }
    }
}

fn existing_instance_responded(endpoint: SocketAddrV4) -> Result<bool, SingleInstanceError> {
    for _ in 0..PROBE_ATTEMPTS {
        let response = send_probe(endpoint, HELLO_MESSAGE)?;
        if response == PRESENT_MESSAGE {
            return Ok(true);
        }
        if response.is_empty() {
            continue;
        }
        return Err(SingleInstanceError::Unavailable(tr!("single-instance IPC endpoint {endpoint} replied with an unexpected payload", endpoint = endpoint).into_owned()));
    }
    Ok(false)
}

fn send_probe(endpoint: SocketAddrV4, payload: &[u8]) -> Result<Vec<u8>, SingleInstanceError> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).map_err(|err| {
        SingleInstanceError::Unavailable(tr!("failed to create single-instance probe socket: {err}", err = err).into_owned())
    })?;
    socket
        .set_read_timeout(Some(PROBE_TIMEOUT))
        .map_err(|err| {
            SingleInstanceError::Unavailable(tr!("failed to configure single-instance probe timeout: {err}", err = err).into_owned())
        })?;
    socket
        .set_write_timeout(Some(PROBE_TIMEOUT))
        .map_err(|err| {
            SingleInstanceError::Unavailable(tr!("failed to configure single-instance probe timeout: {err}", err = err).into_owned())
        })?;
    socket.connect(endpoint).map_err(|err| {
        SingleInstanceError::Unavailable(tr!("failed to connect single-instance probe to {endpoint}: {err}", endpoint = endpoint, err = err).into_owned())
    })?;
    socket.send(payload).map_err(|err| {
        SingleInstanceError::Unavailable(tr!("failed to send single-instance probe to {endpoint}: {err}", endpoint = endpoint, err = err).into_owned())
    })?;

    let mut buffer = [0_u8; 128];
    match socket.recv(&mut buffer) {
        Ok(len) => Ok(buffer[..len].to_vec()),
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Ok(Vec::new())
        }
        Err(err) => Err(SingleInstanceError::Unavailable(tr!("failed to receive single-instance probe response from {endpoint}: {err}", endpoint = endpoint, err = err).into_owned())),
    }
}
