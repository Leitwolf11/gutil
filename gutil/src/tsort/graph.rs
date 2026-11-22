use std::io::{self, Write};
use std::rc::Rc;

use rustc_hash::FxHashMap;
use smallvec::SmallVec;

type NodeId = usize;
const REMOVED_EDGE: NodeId = NodeId::MAX;

struct Node {
    name: Rc<[u8]>,
    incoming: usize,
    successors: SmallVec<[NodeId; 2]>,
}

#[derive(Default)]
pub(super) struct Builder {
    graph: Graph,
    ids: FxHashMap<Rc<[u8]>, NodeId>,
    predecessor: Option<NodeId>,
}

impl Builder {
    pub(super) fn token(&mut self, token: &[u8]) {
        // GNU names terminate at the first NUL even though NUL is not a delimiter.
        let name = &token[..memchr::memchr(0, token).unwrap_or(token.len())];
        let id = if let Some(&id) = self.ids.get(name) {
            id
        } else {
            let id = self.graph.nodes.len();
            let name = Rc::<[u8]>::from(name);
            self.ids.insert(Rc::clone(&name), id);
            self.graph.nodes.push(Node {
                name,
                incoming: 0,
                successors: SmallVec::new(),
            });
            id
        };
        if let Some(predecessor) = self.predecessor.take() {
            if predecessor != id {
                self.graph.nodes[predecessor].successors.push(id);
                self.graph.nodes[id].incoming += 1;
            }
        } else {
            self.predecessor = Some(id);
        }
    }

    pub(super) fn finish(self) -> io::Result<Graph> {
        if self.predecessor.is_some() {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "input contains an odd number of tokens",
            ))
        } else {
            Ok(self.graph)
        }
    }
}

#[derive(Default)]
pub(super) struct Graph {
    nodes: Vec<Node>,
}

#[derive(Clone, Copy)]
enum Visit {
    Unseen,
    Active(usize),
    Done,
}

struct Frame {
    node: NodeId,
    remaining: usize,
}

impl Graph {
    pub(super) fn order(
        &mut self,
        output: &mut impl Write,
        mut report_cycle: impl FnMut(&[&[u8]]),
    ) -> io::Result<bool> {
        let mut queue = Vec::with_capacity(self.nodes.len());
        self.enqueue_roots(&mut queue, 0, |_| true);
        self.drain(&mut queue, 0, output)?;
        if queue.len() == self.nodes.len() {
            return Ok(true);
        }

        let consumed = queue.len();
        let mut visits = vec![Visit::Unseen; self.nodes.len()];
        let mut emitted = vec![false; self.nodes.len()];
        for &id in &queue {
            emitted[id] = true;
            visits[id] = Visit::Done;
        }
        // Deleting DFS back edges makes the remaining graph acyclic.
        self.break_cycles(&mut visits, &mut report_cycle);
        self.enqueue_roots(&mut queue, consumed, |id| !emitted[id]);
        self.drain(&mut queue, consumed, output)?;
        Ok(false)
    }

    fn enqueue_roots(
        &self,
        queue: &mut Vec<NodeId>,
        start: usize,
        include: impl Fn(NodeId) -> bool,
    ) {
        queue.extend(
            self.nodes
                .iter()
                .enumerate()
                .filter_map(|(id, node)| (node.incoming == 0 && include(id)).then_some(id)),
        );
        queue[start..].sort_unstable_by(|&a, &b| self.nodes[a].name.cmp(&self.nodes[b].name));
    }

    fn drain(
        &mut self,
        queue: &mut Vec<NodeId>,
        mut head: usize,
        output: &mut impl Write,
    ) -> io::Result<()> {
        while let Some(&id) = queue.get(head) {
            let node = &mut self.nodes[id];
            output.write_all(&node.name)?;
            output.write_all(b"\n")?;
            let successors = std::mem::take(&mut node.successors);
            // The newest relation comes first when several successors become ready.
            for &next in successors.iter().rev() {
                if next != REMOVED_EDGE {
                    let incoming = &mut self.nodes[next].incoming;
                    *incoming -= 1;
                    if *incoming == 0 {
                        queue.push(next);
                    }
                }
            }
            head += 1;
        }
        Ok(())
    }

    fn break_cycles(&mut self, visits: &mut [Visit], report: &mut impl FnMut(&[&[u8]])) {
        let mut stack = Vec::<Frame>::new();
        for root in 0..self.nodes.len() {
            if !matches!(visits[root], Visit::Unseen) {
                continue;
            }
            self.visit(root, visits, &mut stack);
            while let Some(frame) = stack.last_mut() {
                let Some(index) = frame.remaining.checked_sub(1) else {
                    visits[frame.node] = Visit::Done;
                    stack.pop();
                    continue;
                };
                frame.remaining = index;
                let current = frame.node;
                let next = self.nodes[current].successors[index];
                match visits[next] {
                    Visit::Unseen => self.visit(next, visits, &mut stack),
                    Visit::Active(start) => {
                        let names: Vec<&[u8]> = stack[start..]
                            .iter()
                            .map(|frame| self.nodes[frame.node].name.as_ref())
                            .collect();
                        report(&names);
                        self.nodes[current].successors[index] = REMOVED_EDGE;
                        self.nodes[next].incoming -= 1;
                    }
                    Visit::Done => {}
                }
            }
        }
    }

    fn visit(&self, node: NodeId, visits: &mut [Visit], stack: &mut Vec<Frame>) {
        visits[node] = Visit::Active(stack.len());
        stack.push(Frame {
            node,
            remaining: self.nodes[node].successors.len(),
        });
    }
}
