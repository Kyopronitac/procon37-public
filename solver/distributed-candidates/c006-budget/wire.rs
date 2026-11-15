//! Bounded framed JSON on a private TCP connection. No networking thread.
use crate::planner::{Candidate, Task};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::TcpStream;

pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 4 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
pub enum ClientMessage {
    Authenticate {
        version: u32,
        token: String,
    },
    Solve {
        session: u64,
        request: u64,
        digest: u64,
        budget_ms: u64,
        task: Task,
    },
}
#[derive(Serialize, Deserialize)]
pub enum ServerMessage {
    Ready {
        version: u32,
        total_threads: usize,
        offset: usize,
    },
    Result {
        session: u64,
        request: u64,
        digest: u64,
        started: usize,
        completed: usize,
        elapsed_ms: u64,
        candidate: Option<Candidate>,
    },
    Error {
        message: String,
    },
}

pub struct FramedSocket {
    socket: TcpStream,
    read: Vec<u8>,
    writes: VecDeque<Vec<u8>>,
    written: usize,
    rx_bytes: u64,
    tx_bytes: u64,
}
impl FramedSocket {
    pub fn new(socket: TcpStream) -> io::Result<Self> {
        socket.set_nonblocking(true)?;
        socket.set_nodelay(true)?;
        Ok(Self {
            socket,
            read: Vec::new(),
            writes: VecDeque::new(),
            written: 0,
            rx_bytes: 0,
            tx_bytes: 0,
        })
    }
    pub fn received_bytes(&self) -> u64 {
        self.rx_bytes
    }
    pub fn sent_bytes(&self) -> u64 {
        self.tx_bytes
    }
    pub fn send<T: Serialize>(&mut self, value: &T) -> io::Result<()> {
        let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
        if bytes.len() > MAX_FRAME {
            return Err(io::Error::other("outgoing frame too large"));
        }
        if self.writes.iter().map(Vec::len).sum::<usize>() + bytes.len() > 2 * MAX_FRAME {
            return Err(io::Error::other("outgoing queue full"));
        }
        let mut frame = Vec::with_capacity(4 + bytes.len());
        frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        frame.extend_from_slice(&bytes);
        self.writes.push_back(frame);
        Ok(())
    }
    pub fn poll<T: for<'de> Deserialize<'de>>(&mut self) -> io::Result<Vec<T>> {
        while let Some(bytes) = self.writes.front() {
            match self.socket.write(&bytes[self.written..]) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "closed socket")),
                Ok(count) => {
                    self.tx_bytes += count as u64;
                    self.written += count;
                    if self.written == bytes.len() {
                        self.writes.pop_front();
                        self.written = 0;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        let mut buffer = [0; 16384];
        let mut messages = Vec::new();
        // Bound work per control-loop iteration even for a faulty peer.
        let mut received = 0;
        loop {
            match self.socket.read(&mut buffer) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "peer disconnected",
                    ));
                }
                Ok(count) => {
                    self.rx_bytes += count as u64;
                    self.read.extend_from_slice(&buffer[..count]);
                    received += count;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
            if self.read.len() >= 4
                && u32::from_be_bytes(self.read[..4].try_into().unwrap()) as usize > MAX_FRAME
            {
                return Err(io::Error::other("incoming frame too large"));
            }
            if self.read.len() > MAX_FRAME + 4 {
                return Err(io::Error::other("incoming buffer too large"));
            }
            if received >= 262144 {
                break;
            }
        }
        loop {
            if self.read.len() < 4 {
                break;
            }
            let size = u32::from_be_bytes(self.read[..4].try_into().unwrap()) as usize;
            if size == 0 || size > MAX_FRAME {
                return Err(io::Error::other("invalid frame size"));
            }
            if self.read.len() < size + 4 {
                break;
            }
            let value =
                serde_json::from_slice(&self.read[4..4 + size]).map_err(io::Error::other)?;
            self.read.drain(..4 + size);
            messages.push(value);
            if messages.len() > 16 {
                return Err(io::Error::other("too many frames"));
            }
        }
        Ok(messages)
    }
}

pub fn token_matches(expected: &str, supplied: &str) -> bool {
    let mut mismatch = expected.len() ^ supplied.len();
    for index in 0..expected.len().max(supplied.len()) {
        mismatch |= usize::from(
            expected.as_bytes().get(index).copied().unwrap_or(0)
                ^ supplied.as_bytes().get(index).copied().unwrap_or(0),
        );
    }
    mismatch == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    #[test]
    fn fragmented_frames_and_oversized_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut writer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let mut reader = FramedSocket::new(listener.accept().unwrap().0).unwrap();
        let value = serde_json::json!({"request":42,"payload":"state"});
        let bytes = serde_json::to_vec(&value).unwrap();
        writer
            .write_all(&(bytes.len() as u32).to_be_bytes()[..2])
            .unwrap();
        assert!(reader.poll::<serde_json::Value>().unwrap().is_empty());
        writer
            .write_all(&(bytes.len() as u32).to_be_bytes()[2..])
            .unwrap();
        writer.write_all(&bytes[..3]).unwrap();
        assert!(reader.poll::<serde_json::Value>().unwrap().is_empty());
        writer.write_all(&bytes[3..]).unwrap();
        let mut actual = Vec::new();
        for _ in 0..100 {
            actual.extend(reader.poll::<serde_json::Value>().unwrap());
            if !actual.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(actual, vec![value]);
        writer
            .write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())
            .unwrap();
        let mut failed = false;
        for _ in 0..100 {
            if reader.poll::<serde_json::Value>().is_err() {
                failed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(failed);
    }
    #[test]
    fn authentication_is_exact() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abc", "abc\0"));
        assert!(!token_matches("abc", "abd"));
    }
}
