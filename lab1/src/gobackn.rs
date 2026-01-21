use std::{
    collections::VecDeque,
    sync::mpsc::{self, RecvTimeoutError},
    time::{Duration, Instant},
};

const DATA_SIZE: usize = u8::MAX as usize;
const TIMEOUT: Duration = Duration::from_millis(200);
const TIMEOUT_TOTAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
enum PacketState {
    Begin,
    Ongoing,
    End,
}

type AckNumber = u32;

#[derive(Debug, Clone)]
pub struct Packet {
    number: AckNumber,
    data: [u8; DATA_SIZE],
    size: u8,
    state: PacketState,
}

pub struct Sender {
    tx: mpsc::Sender<Packet>,
    rx: mpsc::Receiver<u32>,
    window_size: AckNumber,
    base: AckNumber,
    packets_total: usize,
    packets_send: usize,
    packets_ack: usize,
    packets_to_send: VecDeque<Packet>,
    packets_to_ack: Vec<bool>,
}

impl Sender {
    pub fn new(
        tx: mpsc::Sender<Packet>,
        rx: mpsc::Receiver<AckNumber>,
        window_size: AckNumber,
    ) -> Self {
        Self {
            tx,
            rx,
            window_size,
            base: 0,
            packets_total: 0,
            packets_send: 0,
            packets_ack: 0,
            packets_to_send: VecDeque::with_capacity(window_size as usize),
            packets_to_ack: vec![false; window_size as usize],
        }
    }

    fn reset(&mut self, message: &str) {
        self.base = 0;
        self.packets_total = message.as_bytes().len().div_ceil(DATA_SIZE).max(2);
        self.packets_send = 0;
        self.packets_ack = 0;
    }

    fn prepare_packets(&mut self, message: &str) {
        let bytes = message.as_bytes();
        let start = self.base as usize;
        let end = (start + self.window_size as usize).min(self.packets_total);
        let unsend_packets = self.packets_to_send.len();
        let packets = (start..end)
            .enumerate()
            .filter(|(i, _)| *i < unsend_packets)
            .map(|(_, number)| {
                let data_start = DATA_SIZE * number;
                let mut data = [0; DATA_SIZE];
                let data_size = match bytes.len().checked_sub(data_start) {
                    Some(data_size) => {
                        let data_size = DATA_SIZE.min(data_size);
                        data[..data_size]
                            .copy_from_slice(&bytes[data_start..(data_start + data_size)]);
                        data_size
                    }
                    None => 0,
                };
                let state = if number == 0 {
                    PacketState::Begin
                } else if number + 1 == self.packets_total {
                    PacketState::End
                } else {
                    PacketState::Ongoing
                };
                Packet {
                    number: number as AckNumber,
                    data,
                    size: data_size as u8,
                    state,
                }
            });
        self.packets_to_send.extend(packets);
    }

    pub fn send(&mut self, message: &str) {
        self.reset(message);
        let time = Instant::now();
        while self.packets_ack < self.packets_total {
            if time.elapsed() > TIMEOUT_TOTAL {
                panic!("Message send timeout");
            }
            self.prepare_packets(message);
            for (i, packet) in self.packets_to_send.iter().enumerate() {
                if let Err(e) = self.tx.send(packet.clone()) {
                    panic!(
                        "Failed to send packet {}, base {}: {e}",
                        packet.number, self.base
                    )
                }
                self.packets_to_ack[i] = false;
                self.packets_send += 1;
            }
            self.ack_packets();
        }
    }

    fn ack_packets(&mut self) {
        let mut time = Instant::now();
        let start = self.base as usize;
        let end = start + self.packets_to_send.len();
        loop {
            match self.rx.recv_timeout(TIMEOUT) {
                Ok(number) => {
                    let number = number as usize;
                    if number >= start && number < end {
                        self.packets_to_ack[number - start] = true;
                        while self.packets_to_ack[self.base as usize - start] {
                            self.base += 1;
                            self.packets_ack += 1;
                            time = Instant::now();
                        }
                    }
                }
                Err(e @ RecvTimeoutError::Disconnected) => {
                    panic!("Failed to receive ACK for packet: {e}");
                }
                Err(RecvTimeoutError::Timeout) => break,
            }
            if time.elapsed() > TIMEOUT {
                break;
            }
        }
    }

    pub fn packets_send(&self) -> usize {
        self.packets_send
    }

    pub fn packets_ack(&self) -> usize {
        self.packets_ack
    }
}

pub struct Reader {
    tx: mpsc::Sender<AckNumber>,
    rx: mpsc::Receiver<Packet>,
    number: AckNumber,
    packets_received: usize,
    packets_ack: usize,
}

impl Reader {
    pub fn new(tx: mpsc::Sender<AckNumber>, rx: mpsc::Receiver<Packet>) -> Self {
        Self {
            tx,
            rx,
            number: 0,
            packets_received: 0,
            packets_ack: 0,
        }
    }

    pub fn read(&mut self) -> String {
        let mut data = Vec::<u8>::new();
        let time_total = Instant::now();
        loop {
            if time_total.elapsed() > TIMEOUT_TOTAL {
                panic!("Message read timeout");
            }
            match self.rx.recv_timeout(TIMEOUT) {
                Ok(packet) => {
                    self.packets_received += 1;
                    if packet.number < self.number {
                        self.send_ack(packet.number);
                        continue;
                    }
                    if packet.number > self.number {
                        continue;
                    }
                    if self.packets_ack == 0 && !matches!(packet.state, PacketState::Begin) {
                        panic!("First packet does not correspond to the start of the message");
                    } else if self.packets_ack != 0 && matches!(packet.state, PacketState::Begin) {
                        panic!("Non first packet corresponds to the start of the message");
                    }
                    data.extend(&packet.data[..packet.size as usize]);
                    self.send_ack(self.number);
                    self.packets_ack += 1;
                    self.number += 1;
                    if matches!(packet.state, PacketState::End) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(e @ RecvTimeoutError::Disconnected) => {
                    panic!("Failed to receive packet: {e}");
                }
            }
        }
        match String::from_utf8(data) {
            Ok(data) => data,
            Err(e) => {
                panic!("Failed to encode the message: {}", e);
            }
        }
    }

    fn send_ack(&mut self, ack: AckNumber) {
        if let Err(e) = self.tx.send(ack) {
            panic!("Failed to send ack {}: {e}", ack);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        io::Read,
        thread::{self},
    };

    use crate::simulate_loss;

    use super::*;

    fn get_file_string() -> String {
        let mut s = String::new();
        File::open("src/gobackn.rs")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        s
    }

    fn setup(window_size: AckNumber, message: &str) -> String {
        let (tx_packet, rx_packet) = mpsc::channel();
        let (tx_ack, rx_ack) = mpsc::channel();
        let mut sender = Sender::new(tx_packet, rx_ack, window_size);
        let mut reader = Reader::new(tx_ack, rx_packet);
        thread::scope(|s| {
            s.spawn(|| {
                sender.send(message);
            });
            reader.read()
        })
    }

    fn setup_loss(window_size: AckNumber, message: &str, loss: f64) -> String {
        let (tx_packet, rx_packet) = mpsc::channel();
        let (tx_ack, rx_ack) = mpsc::channel();
        let (rx_packet, rx_ack, loss_handle) = simulate_loss(rx_packet, rx_ack, loss);
        let mut sender = Sender::new(tx_packet, rx_ack, window_size);
        let mut reader = Reader::new(tx_ack, rx_packet);
        let message_received = thread::scope(|s| {
            s.spawn(|| {
                sender.send(message);
            });
            reader.read()
        });
        loss_handle.join().unwrap();
        assert!(false);
        message_received
    }

    #[test]
    fn test_gobackn_file() {
        let message_send = get_file_string();
        let message_received = setup(3, &message_send);
        assert_eq!(message_send, message_received);
        let message_received = setup(1, &message_send);
        assert_eq!(message_send, message_received);
    }

    #[test]
    fn test_gobackn_file_loss() {
        let message_send = get_file_string();
        let message_received = setup_loss(3, &message_send, 0.0);
        assert_eq!(message_send, message_received);
        // let message_received = setup_loss(3, &message_send, 0.25);
        // assert_eq!(message_send, message_received);
        // let message_received = setup_loss(3, message_send.clone(), 0.5);
        // assert_eq!(message_send, message_received);
    }

    #[test]
    fn test_gobackn_small() {
        let message_send = String::from("test");
        let message_received = setup(5, &message_send);
        assert_eq!(message_send, message_received);
        let message_send = String::from("");
        let message_received = setup(5, &message_send);
        assert_eq!(message_send, message_received);
    }
}
