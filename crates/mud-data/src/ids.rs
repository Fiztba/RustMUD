//! Generational arenas for runtime entities (PLAN.md §3.2).
//!
//! Every cross-reference between chars and objs is a generational ID, so a
//! stale reference fails the liveness check instead of dangling. The
//! deferred-extraction *semantics* (mark, sweep at pulse end)
//! are preserved in db.rs because scripts and combat observe the timing.

/// Generational handle: index + generation. `Copy`, cheap, order-stable.
pub struct Id<T> {
    pub idx: u32,
    pub generation: u32,
    _m: std::marker::PhantomData<fn() -> T>,
}

// Manual impls: derives would bound on `T`, but the tag type is phantom.
impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Id<T> {}
impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx && self.generation == other.generation
    }
}
impl<T> Eq for Id<T> {}
impl<T> std::hash::Hash for Id<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.idx.hash(state);
        self.generation.hash(state);
    }
}
impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.idx, self.generation).cmp(&(other.idx, other.generation))
    }
}
impl<T> std::fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Id({}v{})", self.idx, self.generation)
    }
}

impl<T> Id<T> {
    fn new(idx: u32, generation: u32) -> Self {
        Self { idx, generation, _m: std::marker::PhantomData }
    }
}

pub struct CharTag;
pub struct ObjTag;
pub type CharId = Id<CharTag>;
pub type ObjId = Id<ObjTag>;

enum Slot<T> {
    Free { next_free: Option<u32> },
    Full { generation: u32, value: T },
}

/// Simple generational arena. Slots are recycled LIFO; generations only grow.
pub struct Arena<T, Tag> {
    slots: Vec<Slot<T>>,
    free_head: Option<u32>,
    next_gen: u32,
    len: usize,
    _m: std::marker::PhantomData<Tag>,
}

impl<T, Tag> Default for Arena<T, Tag> {
    fn default() -> Self {
        Self { slots: Vec::new(), free_head: None, next_gen: 1, len: 0, _m: std::marker::PhantomData }
    }
}

impl<T, Tag> Arena<T, Tag> {
    pub fn insert(&mut self, value: T) -> Id<Tag> {
        let generation = self.next_gen;
        self.next_gen = self.next_gen.wrapping_add(1);
        self.len += 1;
        match self.free_head {
            Some(idx) => {
                let next = match &self.slots[idx as usize] {
                    Slot::Free { next_free } => *next_free,
                    Slot::Full { .. } => unreachable!("free list points at full slot"),
                };
                self.free_head = next;
                self.slots[idx as usize] = Slot::Full { generation, value };
                Id::new(idx, generation)
            }
            None => {
                let idx = self.slots.len() as u32;
                self.slots.push(Slot::Full { generation, value });
                Id::new(idx, generation)
            }
        }
    }

    pub fn remove(&mut self, id: Id<Tag>) -> Option<T> {
        let slot = self.slots.get_mut(id.idx as usize)?;
        match slot {
            Slot::Full { generation, .. } if *generation == id.generation => {
                let old = std::mem::replace(slot, Slot::Free { next_free: self.free_head });
                self.free_head = Some(id.idx);
                self.len -= 1;
                match old {
                    Slot::Full { value, .. } => Some(value),
                    Slot::Free { .. } => unreachable!(),
                }
            }
            _ => None,
        }
    }

    pub fn get(&self, id: Id<Tag>) -> Option<&T> {
        match self.slots.get(id.idx as usize) {
            Some(Slot::Full { generation, value }) if *generation == id.generation => Some(value),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, id: Id<Tag>) -> Option<&mut T> {
        match self.slots.get_mut(id.idx as usize) {
            Some(Slot::Full { generation, value }) if *generation == id.generation => Some(value),
            _ => None,
        }
    }

    pub fn contains(&self, id: Id<Tag>) -> bool {
        self.get(id).is_some()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Disjoint mutable borrow of two distinct live slots (fighting pairs,
    /// give/take, etc.). Panics if `a == b`.
    pub fn get2_mut(&mut self, a: Id<Tag>, b: Id<Tag>) -> (Option<&mut T>, Option<&mut T>) {
        assert!(a.idx != b.idx, "get2_mut with identical ids");
        let (lo, hi) = if a.idx < b.idx { (a, b) } else { (b, a) };
        let (left, right) = self.slots.split_at_mut(hi.idx as usize);
        let lo_ref = match left.get_mut(lo.idx as usize) {
            Some(Slot::Full { generation, value }) if *generation == lo.generation => Some(value),
            _ => None,
        };
        let hi_ref = match right.first_mut() {
            Some(Slot::Full { generation, value }) if *generation == hi.generation => Some(value),
            _ => None,
        };
        if a.idx < b.idx { (lo_ref, hi_ref) } else { (hi_ref, lo_ref) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_ids_fail_liveness() {
        let mut a: Arena<i32, CharTag> = Arena::default();
        let id = a.insert(7);
        assert_eq!(a.get(id), Some(&7));
        a.remove(id);
        assert_eq!(a.get(id), None);
        let id2 = a.insert(9);
        assert_eq!(id2.idx, id.idx); // slot recycled
        assert_ne!(id2.generation, id.generation); // but generation differs
        assert_eq!(a.get(id), None);
        assert_eq!(a.get(id2), Some(&9));
    }
}
