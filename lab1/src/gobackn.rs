use std::{
    sync::mpsc::{self, RecvTimeoutError},
    time::{Duration, Instant},
};

const DATA_SIZE: usize = u8::MAX as usize;
const TIMEOUT_MS: u64 = 200;

#[derive(Debug, Clone)]
enum PacketState {
    Begin,
    Ongoing,
    End,
}

type AckNumber = u32;

#[derive(Debug)]
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
    packets_send_total: usize,
    packets_send_ack: usize,
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
            packets_send_total: 0,
            packets_send_ack: 0,
            packets_to_ack: vec![false; window_size as usize],
        }
    }

    pub fn send(&mut self, message: String) {
        let bytes = message.as_bytes();
        let total_packets = bytes.len().div_ceil(DATA_SIZE).max(2);
        while self.packets_send_ack < total_packets {
            let start = self.base as usize;
            let end = (start + self.window_size as usize).min(total_packets);
            for i in start..end {
                let mut data = [0; DATA_SIZE];
                let data_start = DATA_SIZE * i;
                let data_size = match bytes.len().checked_sub(data_start) {
                    Some(data_size) => {
                        let data_size = DATA_SIZE.min(data_size);
                        data[..data_size]
                            .copy_from_slice(&bytes[data_start..(data_start + data_size)]);
                        data_size
                    }
                    None => 0,
                };
                let state = if i == 0 {
                    println!("send beging");
                    PacketState::Begin
                } else if i + 1 == total_packets {
                    println!("send end");
                    PacketState::End
                } else {
                    PacketState::Ongoing
                };
                let packet = Packet {
                    number: i as AckNumber,
                    data,
                    size: data_size as u8,
                    state,
                };
                if let Err(e) = self.tx.send(packet) {
                    panic!("Failed to send packet {i}, base {}: {e}", self.base)
                }
                self.packets_to_ack[i - start] = false;
                self.packets_send_total += 1;
            }
            self.ack_packets(start, end);
        }
    }

    fn ack_packets(&mut self, start: usize, end: usize) {
        let timer = Instant::now();
        let timeout = Duration::from_millis(TIMEOUT_MS);
        loop {
            match self.rx.recv_timeout(timeout) {
                Ok(number) => {
                    let number = number as usize;
                    if number >= start && number < end {
                        self.packets_to_ack[number - start] = true;
                    }
                }
                Err(e @ RecvTimeoutError::Disconnected) => {
                    panic!("Failed to receive ACK for packet: {e}");
                }
                Err(RecvTimeoutError::Timeout) => break,
            }
            if timer.elapsed() > timeout {
                break;
            }
        }
        for _ in self.packets_to_ack.iter().take_while(|&&is_ack| is_ack) {
            self.base += 1;
            self.packets_send_ack += 1;
        }
    }

    pub fn packets_send_total(&self) -> usize {
        self.packets_send_total
    }

    pub fn packets_send_ack(&self) -> usize {
        self.packets_send_ack
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
        let timeout = Duration::from_millis(TIMEOUT_MS);
        loop {
            match self.rx.recv_timeout(timeout) {
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

    fn setup(window_size: AckNumber, message: String) -> String {
        let (tx_packet, rx_packet) = mpsc::channel();
        let (tx_ack, rx_ack) = mpsc::channel();
        let mut sender = Sender::new(tx_packet, rx_ack, window_size);
        let mut reader = Reader::new(tx_ack, rx_packet);
        let sender_handle = thread::spawn(move || {
            sender.send(message);
        });
        let message_received = reader.read();
        sender_handle.join().unwrap();
        message_received
    }

    fn setup_loss(window_size: AckNumber, message: String, loss: f64) -> String {
        let (tx_packet, rx_packet) = mpsc::channel();
        let (tx_ack, rx_ack) = mpsc::channel();
        let (rx_packet, rx_ack, loss_handle) = simulate_loss(rx_packet, rx_ack, loss);
        let mut sender = Sender::new(tx_packet, rx_ack, window_size);
        let mut reader = Reader::new(tx_ack, rx_packet);
        let sender_handle = thread::spawn(move || {
            sender.send(message.clone());
            message
        });
        let message_received = reader.read();
        sender_handle.join().unwrap();
        drop(reader);
        loss_handle.join().unwrap();
        message_received
    }

    #[test]
    fn test_gobackn_file() {
        let message_send = get_file_string();
        let message_received = setup(3, message_send.clone());
        assert_eq!(message_send, message_received);
        let message_received = setup(1, message_send.clone());
        assert_eq!(message_send, message_received);
    }

    #[test]
    fn test_gobackn_file_loss() {
        let message_send = get_file_string();
        let message_received = setup_loss(3, message_send.clone(), 0.0);
        assert_eq!(message_send, message_received);
        let message_received = setup_loss(3, message_send.clone(), 0.25);
        assert_eq!(message_send, message_received);
        // let message_received = setup_loss(3, message_send.clone(), 0.5);
        // assert_eq!(message_send, message_received);
    }

    #[test]
    fn test_gobackn_small() {
        let message_send = String::from("test");
        let message_received = setup(5, message_send.clone());
        assert_eq!(message_send, message_received);
        let message_send = String::from("");
        let message_received = setup(5, message_send.clone());
        assert_eq!(message_send, message_received);
    }
}
