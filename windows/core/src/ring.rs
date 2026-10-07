//! Fixed-capacity ring buffer: no allocation after construction.

#[derive(Clone)]
pub struct Ring<T: Copy + Default, const N: usize> {
    buf: [T; N],
    head: usize, // next write position
    len: usize,
}

impl<T: Copy + Default, const N: usize> Default for Ring<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + Default, const N: usize> Ring<T, N> {
    pub fn new() -> Self {
        Self {
            buf: [T::default(); N],
            head: 0,
            len: 0,
        }
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    pub fn push(&mut self, v: T) {
        self.buf[self.head] = v;
        self.head = (self.head + 1) % N;
        if self.len < N {
            self.len += 1;
        }
    }

    /// Element `i` counted from the oldest (0) to the newest (`len - 1`).
    pub fn get(&self, i: usize) -> Option<T> {
        if i >= self.len {
            return None;
        }
        let start = (self.head + N - self.len) % N;
        Some(self.buf[(start + i) % N])
    }

    pub fn last(&self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            Some(self.buf[(self.head + N - 1) % N])
        }
    }

    pub fn first(&self) -> Option<T> {
        self.get(0)
    }

    /// Mutable access to the newest element.
    pub fn last_mut(&mut self) -> Option<&mut T> {
        if self.len == 0 {
            None
        } else {
            Some(&mut self.buf[(self.head + N - 1) % N])
        }
    }

    /// Iterates oldest → newest.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = T> + ExactSizeIterator + '_ {
        (0..self.len).map(move |i| self.get(i).unwrap_or_default())
    }

    /// Keeps only the newest `keep` elements.
    pub fn truncate_oldest(&mut self, keep: usize) {
        if keep < self.len {
            self.len = keep;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Ring;

    #[test]
    fn wraps_and_orders() {
        let mut r: Ring<u32, 3> = Ring::new();
        assert!(r.is_empty());
        for v in 1..=5 {
            r.push(v);
        }
        assert_eq!(r.len(), 3);
        assert_eq!(r.iter().collect::<Vec<_>>(), vec![3, 4, 5]);
        assert_eq!(r.first(), Some(3));
        assert_eq!(r.last(), Some(5));
        r.truncate_oldest(2);
        assert_eq!(r.iter().collect::<Vec<_>>(), vec![4, 5]);
        *r.last_mut().unwrap() = 9;
        assert_eq!(r.last(), Some(9));
    }
}
