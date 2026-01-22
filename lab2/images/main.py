import networkx as nx
import matplotlib.pyplot as plt

def draw_topology(graph_type, title, ax):
    G = nx.Graph()
    
    # Defining edges based on PDF descriptions
    if graph_type == "linear":
        # Linear: 0-1-2-3-4 (Derived from Fig 1 and text [cite: 134])
        edges = [(0, 1), (1, 2), (2, 3), (3, 4)]
        G.add_edges_from(edges)
        # To match PDF visual style roughly:
        pos = {0: (4, 0), 1: (3, 1), 2: (2, 2), 3: (1, 1), 4: (0, 0)} 
        
    elif graph_type == "ring":
        # Ring: 0-1-2-3-4-0 (Derived from Fig 3 [cite: 158])
        edges = [(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]
        G.add_edges_from(edges)
        pos = nx.circular_layout(G)
        
    elif graph_type == "star":
        # Star: 0 is center, connected to 1,2,3,4 (Derived from Fig 5 [cite: 169])
        edges = [(0, 1), (0, 2), (0, 3), (0, 4)]
        G.add_edges_from(edges)
        # Force center position for visual accuracy
        pos = {0: (0, 0), 1: (1, 1), 2: (1, -1), 3: (-1, -1), 4: (-1, 1)}
    
    # Drawing
    nx.draw_networkx_nodes(G, pos, node_size=700, node_color='lightblue', ax=ax)
    nx.draw_networkx_edges(G, pos, width=2, edge_color='gray', ax=ax)
    nx.draw_networkx_labels(G, pos, font_size=12, font_family="sans-serif", ax=ax)
    
    ax.set_title(title)
    ax.axis('off')

# Generate and save linear topology
fig1, ax1 = plt.subplots(figsize=(5, 5))
draw_topology("linear", "Linear Topology", ax1)
fig1.savefig("linear_topology.png")
print("Saved linear_topology.png")

# Generate and save ring topology
fig2, ax2 = plt.subplots(figsize=(5, 5))
draw_topology("ring", "Ring Topology", ax2)
fig2.savefig("ring_topology.png")
print("Saved ring_topology.png")

# Generate and save star topology
fig3, ax3 = plt.subplots(figsize=(5, 5))
draw_topology("star", "Star Topology", ax3)
fig3.savefig("star_topology.png")
print("Saved star_topology.png")
