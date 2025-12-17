use rand::{self, Rng};
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{self, JoinHandle},
    time::Duration,
};

pub mod gobackn;

fn simulate_loss<A: Send + 'static, B: Send + 'static>(
    ra: Receiver<A>,
    rb: Receiver<B>,
    loss: f64,
) -> (Receiver<A>, Receiver<B>, JoinHandle<()>) {
    assert!(loss >= 0.0);
    assert!(loss <= 1.0);
    let (txa, rxa) = mpsc::channel();
    let (txb, rxb) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut is_ra_alive = true;
        let mut is_rb_alive = true;
        let mut is_did_work;
        let mut rnd = rand::rng();
        loop {
            is_did_work = false;
            if is_ra_alive {
                match ra.try_recv() {
                    Ok(a) => {
                        if rnd.random::<f64>() >= loss && txa.send(a).is_err() {
                            is_ra_alive = false;
                        }
                        is_did_work = true;
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => is_ra_alive = false,
                }
            }
            if is_rb_alive {
                match rb.try_recv() {
                    Ok(b) => {
                        if rnd.random::<f64>() >= loss && txb.send(b).is_err() {
                            is_rb_alive = false;
                        }
                        is_did_work = true;
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => is_rb_alive = false,
                }
            }
            if !is_ra_alive && !is_rb_alive {
                break;
            }
            if !is_did_work {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });
    (rxa, rxb, handle)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    fn setup_loss(loss: f64) -> (usize, usize) {
        let (txa, rxa) = mpsc::channel();
        let (txb, rxb) = mpsc::channel();
        let (rxa, rxb, handle) = simulate_loss(rxa, rxb, loss);
        let timeout = Duration::from_millis(100);
        for i in 0..100 {
            txa.send(i).unwrap();
            txb.send(i).unwrap();
        }
        let mut count_a = 0;
        while rxa.recv_timeout(timeout).is_ok() {
            count_a += 1;
        }
        let mut count_b = 0;
        while rxb.recv_timeout(timeout).is_ok() {
            count_b += 1;
        }
        drop(txa);
        drop(txb);
        handle.join().unwrap();
        (count_a, count_b)
    }

    #[test]
    fn test_loss() {
        let (count_a, count_b) = setup_loss(0.0);
        assert_eq!(count_a, 100);
        assert_eq!(count_b, 100);
        let (count_a, count_b) = setup_loss(1.0);
        assert_eq!(count_a, 0);
        assert_eq!(count_b, 0);
        let (count_a, count_b) = setup_loss(0.25);
        assert!(count_a >= 75);
        assert!(count_a <= 100);
        assert!(count_b >= 75);
        assert!(count_b <= 100);
    }
}
