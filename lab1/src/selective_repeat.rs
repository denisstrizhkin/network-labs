use std::{
    collections::{BTreeMap, VecDeque},
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

#[derive(Debug, Clone)]
struct SenderPacket {
    packet: Packet,
    is_acked: bool,
    last_sent: Option<Instant>,
}

pub struct Sender {
    tx: mpsc::Sender<Packet>,
    rx: mpsc::Receiver<AckNumber>,
    window_size: AckNumber,
    base: AckNumber,
    packets_total: usize,
    packets_send: usize,
    packets_ack: usize,
    window_packets: VecDeque<SenderPacket>,
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
            window_packets: VecDeque::with_capacity(window_size as usize),
            is_debug,
        }
    }

    fn reset(&mut self, message: &str) {
        self.base = 0;
        self.packets_total = message.len().div_ceil(DATA_SIZE).max(2);
        self.packets_send = 0;
        self.packets_ack = 0;
        self.window_packets.clear();
    }

    fn window_end(&self) -> AckNumber {
        (self.base + self.window_size).min(self.packets_total as u32)
    }

    fn prepare_packets(&mut self, message: &str) {
        let bytes = message.as_bytes();
        let end = self.window_end() as usize;
        let current_in_window = self.window_packets.len();
        let next_number = self.base as usize + current_in_window;
        let packets = (next_number..end).map(|number| {
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
            SenderPacket {
                packet: Packet {
                    number: number as AckNumber,
                    data,
                    size: data_size as u8,
                    state,
                },
                is_acked: false,
                last_sent: None,
            }
        });
        self.window_packets.extend(packets);
    }

    fn do_send_packet(
        tx: &mpsc::Sender<Packet>,
        is_debug: bool,
        sender_packet: &mut SenderPacket,
    ) -> Result<(), String> {
        let number = sender_packet.packet.number;
        let size = sender_packet.packet.size;
        let state = sender_packet.packet.state;
        if let Err(e) = tx.send(sender_packet.packet.clone()) {
            return Err(format!("Failed to send packet {number}: {e}"));
        }
        sender_packet.last_sent = Some(Instant::now());
        if is_debug {
            eprintln!(
                "Sender | Send packet: {}, size: {}, state: {:?}",
                number, size, state
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
            for sender_packet in &mut self.window_packets {
                if sender_packet.is_acked {
                    continue;
                }
                let should_send = match sender_packet.last_sent {
                    None => true,
                    Some(last_sent) => last_sent.elapsed() > TIMEOUT,
                };
                if should_send {
                    Self::do_send_packet(&self.tx, self.is_debug, sender_packet)?;
                    self.packets_send += 1;
                }
            }
            self.ack()?;
        }
        Ok(())
    }

    fn ack(&mut self) -> Result<(), String> {
        let end = self.window_end();
        let time = Instant::now();
        while !self.window_packets.is_empty() && (time.elapsed() < Duration::from_millis(10) || self.window_packets[0].is_acked) {
            match self.rx.try_recv() {
                Ok(number) => {
                    if !(number >= self.base && number < end) {
                        continue;
                    }
                    let index = (number - self.base) as usize;
                    if index < self.window_packets.len() && !self.window_packets[index].is_acked {
                        self.window_packets[index].is_acked = true;
                        self.packets_ack += 1;
                        if self.is_debug {
                            eprintln!(
                                "Sender | Ack packet: {}, {} out of {}",
                                number, self.packets_ack, self.packets_total
                            );
                        }
                    }
                }
                Err(TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(1));
                    if !self.window_packets.is_empty() && self.window_packets[0].is_acked {
                        // Slide window
                        self.window_packets.pop_front();
                        self.base += 1;
                    } else {
                        break;
                    }
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
    expected_number: AckNumber,
    window_size: AckNumber,
    packets_read: usize,
    buffer: BTreeMap<AckNumber, Packet>,
    is_debug: bool,
}

impl Reader {
    #[must_use] 
    pub fn new(
        tx: mpsc::Sender<AckNumber>,
        rx: mpsc::Receiver<Packet>,
        window_size: AckNumber,
        is_debug: bool,
    ) -> Self {
        Self {
            tx,
            rx,
            expected_number: 0,
            window_size,
            packets_read: 0,
            buffer: BTreeMap::new(),
            is_debug,
        }
    }

    fn reset(&mut self) {
        self.expected_number = 0;
        self.packets_read = 0;
        self.buffer.clear();
    }

    fn window_end(&self) -> AckNumber {
        self.expected_number + self.window_size
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
                    let number = packet.number;
                    
                    if number < self.expected_number {
                        self.send_ack(number)?;
                        if self.is_debug {
                            eprintln!(
                                "Reader | ReAck packet {}, state: {:?}, at: {}ms",
                                number,
                                packet.state,
                                time.elapsed().as_millis(),
                            );
                        }
                        continue;
                    }

                    if number >= self.window_end() {
                        if self.is_debug {
                            eprintln!("Reader | Packet {} out of window", number);
                        }
                        continue;
                    }

                    // Selective Repeat: Send ACK even if it's out of order
                    self.send_ack(number)?;

                    if let std::collections::btree_map::Entry::Vacant(e) = self.buffer.entry(number) {
                        if number == 0 && !matches!(packet.state, PacketState::Begin) {
                            return Err("First packet does not correspond to the start of the message".to_string());
                        } else if number != 0 && matches!(packet.state, PacketState::Begin) {
                            return Err("Non first packet corresponds to the start of the message".to_string());
                        }
                        e.insert(packet);
                    }

                    // Process buffer
                    while let Some(p) = self.buffer.remove(&self.expected_number) {
                        data.extend(&p.data[..p.size as usize]);
                        if self.is_debug {
                            eprintln!(
                                "Reader | Deliver packet {}, state: {:?}, at {}ms",
                                p.number,
                                p.state,
                                time.elapsed().as_millis()
                            );
                        }
                        if matches!(p.state, PacketState::End) {
                            is_finished_timeout = Some(Instant::now());
                        }
                        self.expected_number += 1;
                    }
                }
                Err(TryRecvError::Empty) => {
                    if is_finished_timeout.is_some_and(|t| t.elapsed() > 2 * TIMEOUT) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
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
    let mut reader = Reader::new(tx_ack, rx_packet, window_size, true);
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
        let mut reader = Reader::new(tx_ack, rx_packet, window_size, false);
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
    fn test_selective_repeat_file() {
        let message_send = get_file_string();
        let message_received = setup(5, &message_send).0;
        assert_eq!(message_send, message_received);
        let message_received = setup(3, &message_send).0;
        assert_eq!(message_send, message_received);
        let message_received = setup(1, &message_send).0;
        assert_eq!(message_send, message_received);
    }

    #[test]
    fn test_selective_repeat_file_loss() {
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
    fn test_selective_repeat_small() {
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
