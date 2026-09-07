//! What a method that changes something promises about the change.
//!
//! A rule about the whole value cannot say this. Round 3 of the A/B vetting
//! measured that directly: of the three bugs planted in this exact type, only
//! one broke `available <= capacity`, and the other two -- a refill of zero
//! silently topping the bucket back up, and a take succeeding one token short
//! -- left it perfectly true. Four of the six bugs across two stateful
//! scenarios were that shape, and none of them could be stated at all.
//!
//! The promises below say what each operation *does*, in terms of what the
//! type's own read-only methods reported before and after. That is the whole
//! idea: a method that mutates is a pure function from the readings before,
//! plus its arguments, to the readings after.

/// A bucket that meters work: it starts full, takes spend tokens, and
/// refills put them back without ever going past the top.
pub struct TokenBucket {
    capacity: u32,
    available: u32,
}

impl TokenBucket {
    #[ply::ensures(|result| result.available() == capacity)]
    #[ply::ensures(|result| result.capacity() == capacity)]
    pub fn new(capacity: u32) -> TokenBucket {
        TokenBucket {
            capacity,
            available: capacity,
        }
    }

    /// The reading a promise talks about. Ordinary API for this type, not a
    /// window opened for the tool.
    pub fn available(&self) -> u32 {
        self.available
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Succeeds exactly when there are enough tokens; a success removes
    /// exactly that many, a failure changes nothing at all, and neither
    /// moves the capacity.
    #[ply::ensures(|result| *result == (old(self.available()) >= tokens))]
    #[ply::ensures(|result| !*result || self.available() == old(self.available()) - tokens)]
    #[ply::ensures(|result| *result || self.available() == old(self.available()))]
    #[ply::ensures(|result| self.capacity() == old(self.capacity()))]
    pub fn try_take(&mut self, tokens: u32) -> bool {
        if self.available >= tokens {
            self.available -= tokens;
            true
        } else {
            false
        }
    }

    /// Adds exactly what was asked for, stopping at the top. That a refill of
    /// nothing changes nothing follows from this; it is not a separate clause.
    #[ply::ensures(|result| self.available()
        == old(self.available()).saturating_add(tokens).min(old(self.capacity())))]
    #[ply::ensures(|result| self.capacity() == old(self.capacity()))]
    pub fn refill(&mut self, tokens: u32) {
        let room = self.capacity - self.available;
        if tokens >= room {
            self.available = self.capacity;
        } else {
            self.available += tokens;
        }
    }
}
