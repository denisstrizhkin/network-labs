use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

// --- Core OSPF Logic and Types ---

type NodeId = usize;
type Weight = u32;

#[derive(Debug, Clone)]
enum Message {
    Hello(NodeId),
    GetNeighbors,
    SetNeighbors(NodeId, Vec<(NodeId, Weight)>),
    SetTopology(HashMap<NodeId, Vec<(NodeId, Weight)>>),
    Data {
        sender: NodeId,
        destination: NodeId,
        path_trace: Vec<NodeId>,
        content: String,
    },
    Disconnect,
}

#[derive(Copy, Clone, Eq, PartialEq)]
struct State {
    cost: Weight,
    node: NodeId,
}

impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .cmp(&self.cost)
            .then_with(|| self.node.cmp(&other.node))
    }
}
impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// Dijkstra's Algorithm: Calculates next hops for all destinations
fn calculate_routing_table(
    my_id: NodeId,
    topology: &HashMap<NodeId, Vec<(NodeId, Weight)>>,
) -> HashMap<NodeId, NodeId> {
    let mut dist: HashMap<NodeId, Weight> = HashMap::new();
    let mut next_hop: HashMap<NodeId, NodeId> = HashMap::new();
    let mut heap = BinaryHeap::new();

    dist.insert(my_id, 0);
    heap.push(State {
        cost: 0,
        node: my_id,
    });

    while let Some(State { cost, node }) = heap.pop() {
        if cost > *dist.get(&node).unwrap_or(&u32::MAX) {
            continue;
        }

        if let Some(neighbors) = topology.get(&node) {
            for &(neighbor, weight) in neighbors {
                let next_state = State {
                    cost: cost + weight,
                    node: neighbor,
                };
                if next_state.cost < *dist.get(&neighbor).unwrap_or(&u32::MAX) {
                    dist.insert(neighbor, next_state.cost);
                    // Determine next hop relative to my_id
                    if node == my_id {
                        next_hop.insert(neighbor, neighbor);
                    } else if let Some(&hop) = next_hop.get(&node) {
                        next_hop.insert(neighbor, hop);
                    }
                    heap.push(next_state);
                }
            }
        }
    }
    next_hop
}

// --- Router Actor ---

struct Router {
    id: NodeId,
    receiver: Receiver<Message>,
    neighbors: HashMap<NodeId, Sender<Message>>,
    dr_link: Sender<Message>,
    routing_table: HashMap<NodeId, NodeId>,
    topology: HashMap<NodeId, Vec<(NodeId, Weight)>>,
}

impl Router {
    fn run(&mut self) {
        // Phase 1: Hello
        for (_nid, tx) in &self.neighbors {
            let _ = tx.send(Message::Hello(self.id));
        }

        let mut discovered_neighbors = Vec::new();

        loop {
            if let Ok(msg) = self.receiver.recv() {
                match msg {
                    Message::Hello(from) => {
                        // In a real scenario, we might measure latency here.
                        // Using weight 1 for simplicity as per standard OSPF hop count.
                        discovered_neighbors.push((from, 1));
                    }
                    Message::GetNeighbors => {
                        let _ = self
                            .dr_link
                            .send(Message::SetNeighbors(self.id, discovered_neighbors.clone()));
                    }
                    Message::SetTopology(graph) => {
                        self.topology = graph;
                        self.routing_table = calculate_routing_table(self.id, &self.topology);
                        // Log similar to PDF "new shortest ways" [cite: 143]
                        // println!("[Router {}] Table Updated: {:?}", self.id, self.routing_table);
                    }
                    Message::Data {
                        sender,
                        destination,
                        mut path_trace,
                        content,
                    } => {
                        // Append self to trace to verify path
                        path_trace.push(self.id);

                        if self.id == destination {
                            // Output format matching "received message from X: [trace]"
                            println!(
                                "[Router {}] received message from {}: {:?}",
                                self.id, sender, path_trace
                            );
                        } else {
                            if let Some(&next_hop) = self.routing_table.get(&destination) {
                                if let Some(link) = self.neighbors.get(&next_hop) {
                                    // Log forwarding (optional, but helps visualize)
                                    // println!("[Router {}] transferred message from {} to {}: {:?}", self.id, sender, next_hop, path_trace);
                                    let _ = link.send(Message::Data {
                                        sender,
                                        destination,
                                        path_trace,
                                        content,
                                    });
                                }
                            } else {
                                println!(
                                    "[Router {}] cannot send message to {}",
                                    self.id, destination
                                );
                            }
                        }
                    }
                    Message::Disconnect => break,
                    _ => {}
                }
            }
        }
    }
}

// --- Designated Router (DR) ---

fn run_dr(node_count: usize, rx: Receiver<Message>, all_nodes: HashMap<NodeId, Sender<Message>>) {
    // Wait for network to stabilize
    thread::sleep(Duration::from_millis(200));

    // Ask for neighbors
    for tx in all_nodes.values() {
        let _ = tx.send(Message::GetNeighbors);
    }

    let mut global_graph = HashMap::new();
    let mut reports = 0;

    while reports < node_count {
        if let Ok(Message::SetNeighbors(id, neighbors)) = rx.recv() {
            global_graph.insert(id, neighbors);
            reports += 1;
        }
    }

    // Broadcast Topology
    for tx in all_nodes.values() {
        let _ = tx.send(Message::SetTopology(global_graph.clone()));
    }
}

// --- Simulation Harness ---

fn run_simulation(
    topology_name: &str,
    edges: Vec<(NodeId, NodeId)>,
    nodes: Vec<NodeId>,
    src: NodeId,
    dst: NodeId,
) {
    println!("\n=== Running Simulation: {} ===", topology_name);

    // Channels
    let (dr_tx, dr_rx) = channel();
    let mut node_txs = HashMap::new();
    let mut node_rxs = HashMap::new();

    for &id in &nodes {
        let (tx, rx) = channel();
        node_txs.insert(id, tx);
        node_rxs.insert(id, rx);
    }

    // Spawn Routers
    let mut handles = Vec::new();
    for &id in &nodes {
        let rx = node_rxs.remove(&id).unwrap();
        let dr_link = dr_tx.clone();

        // Build Neighbor Links based on 'edges' list
        let mut my_neighbors = HashMap::new();
        for &(u, v) in &edges {
            if u == id {
                if let Some(tx) = node_txs.get(&v) {
                    my_neighbors.insert(v, tx.clone());
                }
            } else if v == id {
                if let Some(tx) = node_txs.get(&u) {
                    my_neighbors.insert(u, tx.clone());
                }
            }
        }



        handles.push(thread::spawn(move || {
            let mut router = Router {
                id,
                receiver: rx,
                neighbors: my_neighbors,
                dr_link,
                routing_table: HashMap::new(),
                topology: HashMap::new(),
            };
            router.run();
        }));
    }

    // Spawn DR
    let dr_node_txs = node_txs.clone();
    let dr_handle = thread::spawn(move || {
        run_dr(nodes.len(), dr_rx, dr_node_txs);
    });

    // Allow OSPF convergence
    thread::sleep(Duration::from_millis(500));

    // Send Test Message
    println!("--- Sending Message from {} to {} ---", src, dst);
    if let Some(tx) = node_txs.get(&src) {
        let _ = tx.send(Message::Data {
            sender: src,
            destination: dst,
            path_trace: vec![], // Empty trace to start
            content: "Payload".to_string(),
        });
    }

    // Wait for delivery
    thread::sleep(Duration::from_millis(500));

    // Cleanup
    for tx in node_txs.values() {
        let _ = tx.send(Message::Disconnect);
    }
    for h in handles {
        let _ = h.join();
    }
    let _ = dr_handle.join();
}

fn main() {
    // 1. Linear Topology
    // PDF: 0-1-2-3-4. Test path 0->4. [cite: 134, 147]
    let linear_edges = vec![(0, 1), (1, 2), (2, 3), (3, 4)];
    run_simulation("Linear Topology", linear_edges, vec![0, 1, 2, 3, 4], 0, 4);

    // 2. Ring Topology
    // PDF: 0-1-2-3-4-0. Test path 0->2 (Should go 0->1->2). [cite: 158, 165]
    let ring_edges = vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)];
    run_simulation("Ring Topology", ring_edges, vec![0, 1, 2, 3, 4], 0, 2);

    // 3. Star Topology
    // PDF: 0 is center. Leaves 1,2,3,4. Test 4->3 (Must go 4->0->3). [cite: 169, 177]
    let star_edges = vec![(0, 1), (0, 2), (0, 3), (0, 4)];
    run_simulation("Star Topology", star_edges, vec![0, 1, 2, 3, 4], 4, 3);
}
