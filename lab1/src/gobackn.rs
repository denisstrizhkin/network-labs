use std::{
    collections::VecDeque,
    sync::mpsc::{self, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use crate::simulate_loss;

const DATA_SIZE: usize = u8::MAX as usize;
const TIMEOUT: Duration = Duration::from_millis(200);
const TIMEOUT_TOTAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy)]
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
    is_debug: bool,
}

impl Sender {
    #[must_use] 
    pub fn new(
        tx: mpsc::Sender<Packet>,
        rx: mpsc::Receiver<AckNumber>,
        window_size: AckNumber,
        is_debug: bool,
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
            is_debug,
        }
    }

    fn reset(&mut self, message: &str) {
        self.base = 0;
        self.packets_total = message.len().div_ceil(DATA_SIZE).max(2);
        self.packets_send = 0;
        self.packets_ack = 0;
        self.packets_to_send.clear();
    }

    fn window_end(&self) -> AckNumber {
        (self.base + self.window_size).min(self.packets_total as u32)
    }

    fn prepare_packets(&mut self, message: &str) {
        let bytes = message.as_bytes();
        let start = self.base as usize;
        let end = self.window_end() as usize;
        let current_in_window = self.packets_to_send.len();
        let packets = (start + current_in_window..end).map(|number| {
            let data_start = DATA_SIZE * number;
            let mut data = [0; DATA_SIZE];
            let data_size = match bytes.len().checked_sub(data_start) {
                Some(data_size) => {
                    let data_size = DATA_SIZE.min(data_size);
                    data[..data_size].copy_from_slice(&bytes[data_start..(data_start + data_size)]);
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

    fn send_packet(&mut self, packet: Packet) -> Result<(), String> {
        let number = packet.number;
        let size = packet.size;
        let state = packet.state;
        if let Err(e) = self.tx.send(packet) {
            return Err(format!(
                "Failed to send packet {}, base {}: {e}",
                number, self.base
            ));
        }
        self.packets_send += 1;
        if self.is_debug {
            eprintln!(
                "Sender | Send packet: {}, size: {}, state: {:?}, window_size: {}",
                number, size, state, self.window_size
            );
        }
        Ok(())
    }

    pub fn send(&mut self, message: &str) -> Result<(), String> {
        self.reset(message);
        let time = Instant::now();
        while self.packets_ack < self.packets_total {
            if time.elapsed() > TIMEOUT_TOTAL {
                return Err("Message send timeout".to_string());
            }
            self.prepare_packets(message);
            let packets = self.packets_to_send.clone();
            for packet in packets {
                self.send_packet(packet)?;
            }
            self.ack()?;
        }
        Ok(())
    }

    fn ack(&mut self) -> Result<(), String> {
        let end = self.window_end();
        let time = Instant::now();
        while !self.packets_to_send.is_empty() && time.elapsed() < TIMEOUT {
            match self.rx.try_recv() {
                Ok(number) => {
                    if !(number >= self.base && number < end) {
                        continue;
                    }
                    for _ in self.base..=number {
                        self.packets_to_send.pop_front();
                        self.packets_ack += 1;
                        self.base += 1;
                    }
                    if self.is_debug {
                        eprintln!(
                            "Sender | Ack up to packet: {}, {} out of {}",
                            number, self.packets_ack, self.packets_total
                        );
                    }
                }
                Err(TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(e @ TryRecvError::Disconnected) => {
                    return Err(format!("Failed to receive ACK: {e}"));
                }
            }
        }
        Ok(())
    }

    #[must_use] 
    pub fn efficiency_coefficient(&self) -> f64 {
        self.packets_total as f64 / self.packets_send as f64
    }
}

pub struct Reader {
    tx: mpsc::Sender<AckNumber>,
    rx: mpsc::Receiver<Packet>,
    number: AckNumber,
    packets_read: usize,
    is_debug: bool,
}

impl Reader {
    #[must_use] 
    pub fn new(tx: mpsc::Sender<AckNumber>, rx: mpsc::Receiver<Packet>, is_debug: bool) -> Self {
        Self {
            tx,
            rx,
            number: 0,
            packets_read: 0,
            is_debug,
        }
    }

    fn reset(&mut self) {
        self.number = 0;
        self.packets_read = 0;
    }

    pub fn read(&mut self) -> Result<String, String> {
        self.reset();
        let mut data = Vec::<u8>::new();
        let mut is_finished_timeout: Option<Instant> = None;
        let time = Instant::now();
        loop {
            if time.elapsed() > TIMEOUT_TOTAL {
                if is_finished_timeout.is_none() {
                    return Err("Message read timeout".to_string());
                }
                break;
            }
            match self.rx.try_recv() {
                Ok(packet) => {
                    self.packets_read += 1;
                    if packet.number < self.number {
                        self.send_ack(packet.number)?;
                        if (packet.number + 1 == self.number) && is_finished_timeout.is_some() {
                            is_finished_timeout = Some(Instant::now());
                        }
                        if self.is_debug {
                            eprintln!(
                                "Reader | ReAck packet {}, state: {:?}, at: {}ms",
                                packet.number,
                                packet.state,
                                time.elapsed().as_millis(),
                            );
                        }
                        continue;
                    }
                    if packet.number > self.number {
                        continue;
                    }
                    if self.number == 0 && !matches!(packet.state, PacketState::Begin) {
                        return Err("First packet does not correspond to the start of the message".to_string());
                    } else if self.number != 0 && matches!(packet.state, PacketState::Begin) {
                        return Err("Non first packet corresponds to the start of the message".to_string());
                    }
                    data.extend(&packet.data[..packet.size as usize]);
                    self.send_ack(self.number)?;
                    self.number += 1;
                    if self.is_debug {
                        eprintln!(
                            "Reader | Ack packet {}, state: {:?}, at {}ms",
                            packet.number,
                            packet.state,
                            time.elapsed().as_millis()
                        );
                    }
                    if matches!(packet.state, PacketState::End) {
                        is_finished_timeout = Some(Instant::now());
                    }
                }
                Err(TryRecvError::Empty) => {
                    if is_finished_timeout.is_some_and(|t| t.elapsed() > 2 * TIMEOUT) {
                        break;
                    }
                }
                Err(e @ TryRecvError::Disconnected) => {
                    return Err(format!("Failed to receive packet: {e}"));
                }
            }
        }
        if self.is_debug {
            eprintln!(
                "Reader | Finished receiving the message at {}ms",
                time.elapsed().as_millis()
            );
        }
        String::from_utf8(data).map_err(|e| format!("Failed to encode the message: {e}"))
    }

    fn send_ack(&mut self, ack: AckNumber) -> Result<(), String> {
        self.tx
            .send(ack)
            .map_err(|e| format!("Failed to send ack {ack}: {e}"))
    }
}

#[must_use] 
pub fn setup(window_size: AckNumber, message: &str) -> (String, f64) {
    let (tx_packet, rx_packet) = mpsc::channel();
    let (tx_ack, rx_ack) = mpsc::channel();
    let mut sender = Sender::new(tx_packet, rx_ack, window_size, true);
    let mut reader = Reader::new(tx_ack, rx_packet, true);
    let message_read = thread::scope(|s| {
        s.spawn(|| {
            if let Err(e) = sender.send(message) {
                eprintln!("Sender | {e}");
            }
        });
        reader.read()
    });
    (message_read.unwrap(), sender.efficiency_coefficient())
}

#[must_use] 
pub fn silent_setup_loss(window_size: AckNumber, message: &str, loss: f64) -> (String, f64) {
    let (tx_packet, rx_packet) = mpsc::channel();
    let (tx_ack, rx_ack) = mpsc::channel();
    let (rx_packet, rx_ack, loss_handle) = simulate_loss(rx_packet, rx_ack, loss);
    let result = {
        let mut sender = Sender::new(tx_packet, rx_ack, window_size, false);
        let mut reader = Reader::new(tx_ack, rx_packet, false);
        let message_read = thread::scope(|s| {
            s.spawn(|| {
                if let Err(e) = sender.send(message) {
                    eprintln!("Sender | {e}");
                }
            });
            reader.read().unwrap_or_else(|e| {
                eprintln!("Reader warning: {e}");
                String::new()
            })
        });
        (message_read, sender.efficiency_coefficient())
    };
    loss_handle.join().unwrap();
    result
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Read};

    use super::*;

    fn get_file_string() -> String {
        let mut s = String::new();
        File::open("src/lib.rs")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        s
    }

    #[test]
    fn test_gobackn_file() {
        let message_send = get_file_string();
        let message_received = setup(5, &message_send).0;
        assert_eq!(message_send, message_received);
        let message_received = setup(3, &message_send).0;
        assert_eq!(message_send, message_received);
        let message_received = setup(1, &message_send).0;
        assert_eq!(message_send, message_received);
    }

    #[test]
    fn test_gobackn_file_loss() {
        let message_send = get_file_string();
        let message_received = setup_loss(3, &message_send, 0.0).0;
        assert_eq!(message_send, message_received);
        let message_received = setup_loss(3, &message_send, 0.25).0;
        assert_eq!(message_send, message_received);
        let message_received = setup_loss(3, &message_send, 0.5).0;
        assert_eq!(message_send, message_received);
        let message_received = setup_loss(3, &message_send, 0.75).0;
        assert_eq!(message_send, message_received);
    }

    #[test]
    fn test_gobackn_small() {
        let message_send = String::from("test");
        let message_received = setup(5, &message_send).0;
        assert_eq!(message_send, message_received);
        let message_send = String::from("");
        let message_received = setup(5, &message_send).0;
        assert_eq!(message_send, message_received);
        let message_send = String::from("test");
        let message_received = setup(1, &message_send).0;
        assert_eq!(message_send, message_received);
        let message_send = String::from("");
        let message_received = setup(1, &message_send).0;
        assert_eq!(message_send, message_received);
    }
}
