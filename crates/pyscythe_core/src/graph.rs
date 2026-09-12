//! Small graph algorithms over node indices.

/// Per-node bookkeeping for Tarjan's algorithm.
struct Node {
    /// Discovery order, once visited.
    index: Option<usize>,
    /// Smallest discovery index reachable from here.
    low: usize,
    on_stack: bool,
}

/// Tarjan's strongly connected components over an adjacency list, iterative
/// so deep graphs cannot overflow the stack. Each component's nodes are sorted.
#[must_use]
pub fn strongly_connected_components(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut nodes: Vec<Node> = edges
        .iter()
        .map(|_| Node {
            index: None,
            low: 0,
            on_stack: false,
        })
        .collect();
    let mut stack: Vec<usize> = Vec::new();
    let mut counter = 0;
    let mut components = Vec::new();

    for root in 0..edges.len() {
        if nodes.get(root).is_none_or(|node| node.index.is_some()) {
            continue;
        }
        let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
        discover(&mut nodes, root, &mut counter, &mut stack);

        while let Some(&mut (node, ref mut next)) = frames.last_mut() {
            if let Some(&target) = edges.get(node).and_then(|targets| targets.get(*next)) {
                *next += 1;
                match nodes.get(target).map(|n| (n.index, n.on_stack)) {
                    Some((None, _)) => {
                        discover(&mut nodes, target, &mut counter, &mut stack);
                        frames.push((target, 0));
                    }
                    Some((Some(target_index), true)) => {
                        if let Some(current) = nodes.get_mut(node)
                            && target_index < current.low
                        {
                            current.low = target_index;
                        }
                    }
                    _ => {}
                }
                continue;
            }
            frames.pop();
            let node_low = nodes.get(node).map_or(0, |n| n.low);
            if let Some(&(parent, _)) = frames.last()
                && let Some(parent_node) = nodes.get_mut(parent)
                && node_low < parent_node.low
            {
                parent_node.low = node_low;
            }
            if nodes.get(node).is_some_and(|n| n.index == Some(n.low)) {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    if let Some(popped) = nodes.get_mut(member) {
                        popped.on_stack = false;
                    }
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                component.sort_unstable();
                components.push(component);
            }
        }
    }
    components
}

/// Marks `node` visited with the next discovery index and pushes it.
fn discover(nodes: &mut [Node], node: usize, counter: &mut usize, stack: &mut Vec<usize>) {
    if let Some(entry) = nodes.get_mut(node) {
        entry.index = Some(*counter);
        entry.low = *counter;
        entry.on_stack = true;
        *counter += 1;
        stack.push(node);
    }
}

#[cfg(test)]
mod tests {
    use super::strongly_connected_components;

    #[test]
    fn finds_cycles_and_singletons() {
        let edges = vec![vec![1], vec![2], vec![0], vec![0]];
        let mut components = strongly_connected_components(&edges);
        components.sort();
        assert_eq!(components, [vec![0, 1, 2], vec![3]]);
    }
}
