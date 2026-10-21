//! Incremental, deterministic A* over one [`ClassGrid`]. One search runs at
//! a time; it can be suspended when the tick's step budget runs out and
//! resumed on a later tick. Costs and the heuristic follow TA (Nanolathe
//! `04` §7, R-PATH-01 §3); see `docs/pathfinding.md`.

use super::grid::{BLOCKED, ClassGrid, STEEP};
use super::{
    HEURISTIC_RADIUS, HEURISTIC_WEIGHT, SHORT_RUN, SHORT_RUN_PENALTY, STEEP_COST, STEP_COST,
    TURN_PENALTY,
};

/// Neighbour offsets in TA's direction order: N, NW, W, SW, S, SE, E, NE
/// (`+z` is south). Odd indices are diagonals.
pub const DIRS: [(i32, i32); 8] = [
    (0, -1),
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
    (1, 0),
    (1, -1),
];

const OPEN: u8 = 1;
const CLOSED: u8 = 2;

/// Reusable per-cell working arrays (sized to the largest grid). Entries
/// are valid only where `stamp == epoch`, so nothing is cleared between
/// searches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scratch {
    epoch: u32,
    stamp: Vec<u32>,
    state: Vec<u8>,
    g: Vec<u32>,
    dir: Vec<u8>,
    run: Vec<u8>,
}

/// Heap entry, ordered by `(f, h, seq)`: lowest total cost, then closest
/// to the goal, then first pushed. A total order, so pops are deterministic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Entry {
    f: u32,
    h: u32,
    seq: u32,
    cell: u32,
}

/// A search in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub start: u32,
    pub goal: u32,
    /// Heap pops so far (with start/goal and the grid, this determines the
    /// whole search state; it is what gets hashed).
    pub pops: u32,
    heap: Vec<Entry>,
    seq: u32,
    goal_xz: (i32, i32),
}

/// What a slice of work ended with.
pub enum Progress {
    /// Budget ran out; call again later.
    Suspended,
    /// Path found: anchor indices from start to goal.
    Found(Vec<u32>),
    /// The heap emptied without reaching the goal (cannot happen when the
    /// goal is in the start's component; kept as a safe exit).
    Exhausted,
}

/// TA's point-goal heuristic: octile `18 * max + 7 * min` less the goal
/// radius, weighted by 3/2.
fn heuristic(a: (i32, i32), goal: (i32, i32)) -> u32 {
    let (dx, dz) = ((a.0 - goal.0).unsigned_abs(), (a.1 - goal.1).unsigned_abs());
    let oct = 18 * dx.max(dz) + 7 * dx.min(dz);
    oct.saturating_sub(HEURISTIC_RADIUS)
        .saturating_mul(HEURISTIC_WEIGHT.0)
        / HEURISTIC_WEIGHT.1
}

impl Search {
    /// Seed a search from `start` to `goal` (anchor indices in `grid`),
    /// with the start's travel direction from the unit's heading sector.
    pub fn new(
        grid: &ClassGrid,
        scratch: &mut Scratch,
        start: u32,
        goal: u32,
        start_dir: u8,
    ) -> Search {
        let n = grid.len();
        if scratch.stamp.len() < n {
            scratch.stamp.resize(n, 0);
            scratch.state.resize(n, 0);
            scratch.g.resize(n, 0);
            scratch.dir.resize(n, 0);
            scratch.run.resize(n, 0);
        }
        scratch.epoch = scratch.epoch.wrapping_add(1);
        if scratch.epoch == 0 {
            scratch.stamp.iter_mut().for_each(|s| *s = 0);
            scratch.epoch = 1;
        }
        let s = start as usize;
        scratch.stamp[s] = scratch.epoch;
        scratch.state[s] = OPEN;
        scratch.g[s] = 0;
        scratch.dir[s] = start_dir & 7;
        scratch.run[s] = 100;
        let goal_xz = grid.coords(goal);
        let h = heuristic(grid.coords(start), goal_xz);
        Search {
            start,
            goal,
            pops: 0,
            heap: vec![Entry {
                f: h,
                h,
                seq: 0,
                cell: start,
            }],
            seq: 1,
            goal_xz,
        }
    }

    /// Run at most `budget` pops. Returns the progress and the pops spent.
    pub fn run(&mut self, grid: &ClassGrid, scratch: &mut Scratch, budget: u32) -> (Progress, u32) {
        let epoch = scratch.epoch;
        let mut spent = 0;
        while spent < budget {
            let Some(e) = self.pop() else {
                return (Progress::Exhausted, spent);
            };
            spent += 1;
            self.pops = self.pops.saturating_add(1);
            let c = e.cell as usize;
            if scratch.stamp[c] != epoch
                || scratch.state[c] == CLOSED
                || e.f.saturating_sub(e.h) != scratch.g[c]
            {
                continue; // stale entry
            }
            scratch.state[c] = CLOSED;
            if e.cell == self.goal {
                return (Progress::Found(self.reconstruct(grid, scratch)), spent);
            }
            let here = grid.coords(e.cell);
            let (g, cur, run) = (scratch.g[c], scratch.dir[c], scratch.run[c]);
            for (d, &(ox, oz)) in DIRS.iter().enumerate() {
                let n = (here.0 + ox, here.1 + oz);
                let tier = grid.tier(n);
                if tier == BLOCKED {
                    continue;
                }
                if d % 2 == 1
                    && (grid.tier((here.0 + ox, here.1)) == BLOCKED
                        || grid.tier((here.0, here.1 + oz)) == BLOCKED)
                {
                    continue; // no corner cutting
                }
                let Some(ni) = grid.index(n) else { continue };
                let ni = ni as usize;
                let touched = scratch.stamp[ni] == epoch;
                if touched && scratch.state[ni] == CLOSED {
                    continue;
                }
                let d8 = d as u8;
                let mut cost = g
                    .saturating_add(TURN_PENALTY[usize::from(d8.wrapping_sub(cur) & 7)])
                    .saturating_add(STEP_COST[d % 2])
                    .saturating_add(if tier == STEEP { STEEP_COST } else { 0 });
                if d8 != cur && run < SHORT_RUN {
                    cost = cost.saturating_add(SHORT_RUN_PENALTY);
                }
                if touched && cost >= scratch.g[ni] {
                    continue; // equal cost never re-parents
                }
                scratch.stamp[ni] = epoch;
                scratch.state[ni] = OPEN;
                scratch.g[ni] = cost;
                scratch.dir[ni] = d8;
                scratch.run[ni] = if d8 == cur { run.saturating_add(1) } else { 1 };
                let h = heuristic(n, self.goal_xz);
                self.push(Entry {
                    f: cost.saturating_add(h),
                    h,
                    seq: self.seq,
                    cell: ni as u32,
                });
                self.seq = self.seq.wrapping_add(1);
            }
        }
        (Progress::Suspended, spent)
    }

    fn reconstruct(&self, grid: &ClassGrid, scratch: &Scratch) -> Vec<u32> {
        let mut out = vec![self.goal];
        let mut c = self.goal;
        let mut guard = grid.len();
        while c != self.start && guard > 0 {
            guard -= 1;
            let (ox, oz) = DIRS[usize::from(scratch.dir[c as usize] & 7)];
            let (x, z) = grid.coords(c);
            match grid.index((x - ox, z - oz)) {
                Some(p) => c = p,
                None => break,
            }
            out.push(c);
        }
        out.reverse();
        out
    }

    fn push(&mut self, e: Entry) {
        self.heap.push(e);
        let mut i = self.heap.len() - 1;
        while i > 0 {
            let p = (i - 1) / 2;
            if self.heap[i] < self.heap[p] {
                self.heap.swap(i, p);
                i = p;
            } else {
                break;
            }
        }
    }

    fn pop(&mut self) -> Option<Entry> {
        if self.heap.is_empty() {
            return None;
        }
        let last = self.heap.len() - 1;
        self.heap.swap(0, last);
        let top = self.heap.pop();
        let n = self.heap.len();
        let mut i = 0;
        loop {
            let (l, r) = (2 * i + 1, 2 * i + 2);
            let mut m = i;
            if l < n && self.heap[l] < self.heap[m] {
                m = l;
            }
            if r < n && self.heap[r] < self.heap[m] {
                m = r;
            }
            if m == i {
                break;
            }
            self.heap.swap(i, m);
            i = m;
        }
        top
    }
}
