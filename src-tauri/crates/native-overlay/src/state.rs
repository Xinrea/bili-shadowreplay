//! Time indexing and byte accounting are independent of Skia.
use std::collections::{BTreeMap, HashMap};

pub(crate) struct Timeline {
    times: Vec<(f64, f64)>,
    next: usize,
    active: Vec<usize>,
    previous: f64,
}

impl Timeline {
    /// Input must be sorted by start time. IDs are positions in that input.
    pub fn new(times: Vec<(f64, f64)>) -> Self {
        Self {
            times,
            next: 0,
            active: Vec::new(),
            previous: f64::NEG_INFINITY,
        }
    }

    pub fn at(&mut self, time: f64) -> &[usize] {
        if time < self.previous {
            // A backwards preview seek rebuilds once; sequential frames never
            // revisit historical events.
            self.next = 0;
            self.active.clear();
        }
        self.active.retain(|id| self.times[*id].1 > time);
        while self.next < self.times.len() && self.times[self.next].0 <= time {
            if self.times[self.next].1 > time {
                self.active.push(self.next);
            }
            self.next += 1;
        }
        self.previous = time;
        &self.active
    }
}

pub(crate) struct ByteLru<T> {
    entries: HashMap<usize, (T, usize, u64)>,
    age: BTreeMap<u64, usize>,
    clock: u64,
    bytes: usize,
    limit: usize,
}

impl<T: Clone> ByteLru<T> {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: HashMap::new(),
            age: BTreeMap::new(),
            clock: 0,
            bytes: 0,
            limit,
        }
    }

    pub fn get(&mut self, id: usize) -> Option<T> {
        let (value, _, old) = self.entries.get_mut(&id)?;
        self.age.remove(old);
        self.clock += 1;
        *old = self.clock;
        self.age.insert(*old, id);
        Some(value.clone())
    }

    pub fn insert(&mut self, id: usize, value: T, bytes: usize) {
        if let Some((_, old_bytes, age)) = self.entries.remove(&id) {
            self.bytes -= old_bytes;
            self.age.remove(&age);
        }
        // Oversized sprites can be used once without retaining them.
        if bytes > self.limit {
            return;
        }
        while self.bytes + bytes > self.limit {
            let Some((_, oldest)) = self.age.pop_first() else {
                break;
            };
            if let Some((_, size, _)) = self.entries.remove(&oldest) {
                self.bytes -= size;
            }
        }
        self.clock += 1;
        self.age.insert(self.clock, id);
        self.entries.insert(id, (value, bytes, self.clock));
        self.bytes += bytes;
    }
}

/// ASS keypad alignment: horizontal and vertical anchor fractions.
pub(crate) fn alignment(value: u8) -> (f32, f32) {
    (
        ((value - 1) % 3) as f32 / 2.0,
        (2 - (value - 1) / 3) as f32 / 2.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_open_times_and_backwards_seeks() {
        let mut t = Timeline::new(vec![(0.0, 2.0), (1.0, 3.0), (3.0, 4.0)]);
        assert_eq!(t.at(-1.0), &[]);
        assert_eq!(t.at(1.0), &[0, 1]);
        assert_eq!(t.at(2.0), &[1]);
        assert_eq!(t.at(3.0), &[2]);
        assert_eq!(t.at(9.0), &[]);
        assert_eq!(t.at(0.0), &[0]);
        assert_eq!(t.at(1.0), &[0, 1]);
    }

    #[test]
    fn lru_is_byte_bounded_and_refreshes_on_read() {
        let mut c = ByteLru::new(8);
        c.insert(0, "a", 4);
        c.insert(1, "b", 4);
        assert_eq!(c.get(0), Some("a"));
        c.insert(2, "c", 4);
        assert_eq!(c.get(1), None);
        assert_eq!(c.bytes, 8);
        c.insert(3, "too large", 9);
        assert_eq!(c.get(3), None);
        c.insert(0, "replacement", 1);
        assert_eq!(c.bytes, 5);
        for _ in 0..1000 {
            c.get(0);
        }
        assert_eq!(c.age.len(), c.entries.len());
    }

    #[test]
    fn keypad_anchors() {
        assert_eq!(alignment(1), (0.0, 1.0));
        assert_eq!(alignment(2), (0.5, 1.0));
        assert_eq!(alignment(4), (0.0, 0.5));
        assert_eq!(alignment(5), (0.5, 0.5));
        assert_eq!(alignment(9), (1.0, 0.0));
    }
}
