pub struct PTIter<It: Iterator> {
    inner: It,
    i: usize,
    cap: usize,
}

impl<It: Iterator> PTIter<It> {
    pub fn new(inner: It, cap: usize) -> Self {
        Self { inner, cap, i: 0 }
    }
}

impl<It: Iterator> Iterator for PTIter<It> {
    type Item = It::Item;

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.cap.saturating_sub(self.i), Some(self.cap))
    }

    fn next(&mut self) -> Option<Self::Item> {
        if self.i >= self.cap {
            return None;
        }
        self.i += 1;
        self.inner.next()
    }
}

impl<It: Iterator> ExactSizeIterator for PTIter<It> {}
