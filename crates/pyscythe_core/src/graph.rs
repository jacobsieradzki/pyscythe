//! Small graph algorithms over node indices.

/// Tarjan's strongly connected components over an adjacency list, iterative
/// so deep graphs cannot overflow the stack. Each component's nodes are sorted.
#[must_use]
pub fn strongly_connected_components(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = edges.len();
    let mut index_of = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut counter = 0;
    let mut components = Vec::new();

    for root in 0..n {
        if index_of[root] != usize::MAX {
            continue;
        }
        let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
        index_of[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;

        while let Some(&mut (node, ref mut next)) = frames.last_mut() {
            if let Some(&target) = edges.get(node).and_then(|targets| targets.get(*next)) {
                *next += 1;
                if index_of[target] == usize::MAX {
                    index_of[target] = counter;
                    low[target] = counter;
                    counter += 1;
                    stack.push(target);
                    on_stack[target] = true;
                    frames.push((target, 0));
                } else if on_stack[target] && index_of[target] < low[node] {
                    low[node] = index_of[target];
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last()
                && low[node] < low[parent]
            {
                low[parent] = low[node];
            }
            if low[node] == index_of[node] {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
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
