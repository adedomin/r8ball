// Copyright (C) 2021  Anthony DeDominic <adedomin@gmail.com>

// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.

// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

fn find_eom(buf: &[u8]) -> Option<usize> {
    buf.iter().position(|&chr| chr == b'\n' || chr == b'\r')
}

fn find_start(buf: &[u8]) -> Option<usize> {
    buf.iter().position(|&chr| chr != b'\n' && chr != b'\r')
}

/// Describes the fullness of the message being returned from the iterator
/// This can be used when a read that returns is not a fully terminated IRC
/// message.
pub enum TruncStatus<T> {
    Full(T),
    Part(T),
}

/// An iterator that will return all valid IRC messages as slices of bytes.
pub struct BufIterator<'a> {
    read_head: usize,
    buffer: &'a [u8],
}

impl<'a> BufIterator<'a> {
    /// Construct a valid iterator for a given buffer.
    /// Make sure to pass a slice of the amount read from an IRCd if you are
    /// reuisng a buffer.
    pub fn new(buffer: &'a [u8]) -> Self {
        BufIterator {
            read_head: 0,
            buffer,
        }
    }
}

impl<'a> Iterator for BufIterator<'a> {
    type Item = TruncStatus<&'a [u8]>;
    fn next(&mut self) -> Option<Self::Item> {
        let buf: &'a [u8] = &self.buffer[self.read_head..];
        let start = match find_start(buf) {
            Some(start) => start,
            None => return None,
        };

        // remove leading delimiter.
        self.read_head += start;
        let buf = &buf[start..];

        if let Some(eom) = find_eom(buf) {
            self.read_head += eom + 1;
            Some(TruncStatus::Full(&buf[..eom]))
        } else {
            self.read_head = self.buffer.len();
            Some(TruncStatus::Part(&buf))
        }
    }
}

#[cfg(test)]
mod test {
    use super::{BufIterator, TruncStatus};

    #[test]
    fn test_iter_simple() {
        // blank lines should be ignored as being junk
        let test_body: &[u8] = b":test 1 2 43

:dsafdsa "; // trailing are indeterminate messages.
        let iter = BufIterator::new(test_body);

        for line in iter {
            match line {
                TruncStatus::Full(x) => {
                    assert_eq!(x, b":test 1 2 43")
                }
                TruncStatus::Part(x) => {
                    assert_eq!(x, b":dsafdsa ")
                }
            }
        }
    }

    #[test]
    fn test_iter_no_partial() {
        // blank lines should be ignored as being junk
        let test_body: &[u8] = b":test 1 2 43\r\n";
        let iter = BufIterator::new(test_body);

        for line in iter {
            match line {
                TruncStatus::Full(x) => {
                    assert_eq!(x, b":test 1 2 43")
                }
                TruncStatus::Part(x) => {
                    assert_eq!(x.len(), 0);
                }
            }
        }
    }
}
