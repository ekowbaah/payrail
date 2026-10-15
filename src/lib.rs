//! Payment orchestration primitives with deterministic money and balanced journals.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Currency(String);

impl Currency {
    pub fn new(code: &str) -> Result<Self, PaymentError> {
        if code.len() == 3 && code.bytes().all(|c| c.is_ascii_uppercase()) {
            Ok(Self(code.to_owned()))
        } else {
            Err(PaymentError::InvalidCurrency)
        }
    }
    pub fn code(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Money {
    pub minor: i64,
    pub currency: Currency,
}

impl Money {
    pub fn new(minor: i64, currency: Currency) -> Result<Self, PaymentError> {
        if minor < 0 {
            Err(PaymentError::NegativeAmount)
        } else {
            Ok(Self { minor, currency })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    pub account: String,
    pub debit: i64,
    pub credit: i64,
    pub currency: Currency,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub reference: String,
    pub postings: Vec<Posting>,
}

impl JournalEntry {
    pub fn validate(&self) -> Result<(), PaymentError> {
        if self.reference.trim().is_empty() || self.postings.len() < 2 {
            return Err(PaymentError::InvalidJournal);
        }
        let currency = &self.postings[0].currency;
        let mut debits = 0i64;
        let mut credits = 0i64;
        for posting in &self.postings {
            if &posting.currency != currency
                || posting.debit < 0
                || posting.credit < 0
                || (posting.debit > 0) == (posting.credit > 0)
            {
                return Err(PaymentError::InvalidJournal);
            }
            debits = debits
                .checked_add(posting.debit)
                .ok_or(PaymentError::Overflow)?;
            credits = credits
                .checked_add(posting.credit)
                .ok_or(PaymentError::Overflow)?;
        }
        if debits != credits {
            return Err(PaymentError::UnbalancedJournal);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentState {
    Created,
    Authorized,
    Captured,
    Failed,
    Refunded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payment {
    pub id: String,
    pub amount: Money,
    pub state: PaymentState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentError {
    InvalidCurrency,
    NegativeAmount,
    InvalidJournal,
    UnbalancedJournal,
    Overflow,
    DuplicateReference,
    DuplicateIdempotencyKey,
    PaymentNotFound,
    InvalidTransition,
}

#[derive(Default)]
pub struct PaymentEngine {
    payments: HashMap<String, Payment>,
    idempotency: HashMap<String, String>,
    journal_references: HashSet<String>,
    journal: Vec<JournalEntry>,
}

impl PaymentEngine {
    pub fn create(&mut self, key: &str, id: &str, amount: Money) -> Result<&Payment, PaymentError> {
        if let Some(existing_id) = self.idempotency.get(key) {
            return self
                .payments
                .get(existing_id)
                .ok_or(PaymentError::PaymentNotFound);
        }
        if self.payments.contains_key(id) {
            return Err(PaymentError::DuplicateIdempotencyKey);
        }
        self.idempotency.insert(key.to_owned(), id.to_owned());
        self.payments.insert(
            id.to_owned(),
            Payment {
                id: id.to_owned(),
                amount,
                state: PaymentState::Created,
            },
        );
        Ok(self.payments.get(id).expect("inserted payment must exist"))
    }
    pub fn transition(&mut self, id: &str, target: PaymentState) -> Result<&Payment, PaymentError> {
        let payment = self
            .payments
            .get_mut(id)
            .ok_or(PaymentError::PaymentNotFound)?;
        let allowed = matches!(
            (payment.state, target),
            (
                PaymentState::Created,
                PaymentState::Authorized | PaymentState::Failed
            ) | (
                PaymentState::Authorized,
                PaymentState::Captured | PaymentState::Failed
            ) | (PaymentState::Captured, PaymentState::Refunded)
        );
        if !allowed {
            return Err(PaymentError::InvalidTransition);
        }
        payment.state = target;
        Ok(payment)
    }
    pub fn post(&mut self, entry: JournalEntry) -> Result<(), PaymentError> {
        entry.validate()?;
        if !self.journal_references.insert(entry.reference.clone()) {
            return Err(PaymentError::DuplicateReference);
        }
        self.journal.push(entry);
        Ok(())
    }
    pub fn payment(&self, id: &str) -> Option<&Payment> {
        self.payments.get(id)
    }
    pub fn journal(&self) -> &[JournalEntry] {
        &self.journal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sek() -> Currency {
        Currency::new("SEK").unwrap()
    }
    #[test]
    fn idempotent_creation_returns_same_payment() {
        let mut e = PaymentEngine::default();
        let amount = Money::new(1250, sek()).unwrap();
        e.create("request-1", "pay-1", amount.clone()).unwrap();
        let again = e.create("request-1", "different-id", amount).unwrap();
        assert_eq!(again.id, "pay-1");
        assert_eq!(e.payments.len(), 1);
    }
    #[test]
    fn rejects_invalid_transition() {
        let mut e = PaymentEngine::default();
        e.create("k", "p", Money::new(100, sek()).unwrap()).unwrap();
        assert_eq!(
            e.transition("p", PaymentState::Captured),
            Err(PaymentError::InvalidTransition)
        );
    }
    #[test]
    fn records_balanced_journal_once() {
        let mut e = PaymentEngine::default();
        let entry = JournalEntry {
            reference: "capture:p".into(),
            postings: vec![
                Posting {
                    account: "processor-clearing".into(),
                    debit: 100,
                    credit: 0,
                    currency: sek(),
                },
                Posting {
                    account: "merchant-payable".into(),
                    debit: 0,
                    credit: 100,
                    currency: sek(),
                },
            ],
        };
        e.post(entry.clone()).unwrap();
        assert_eq!(e.post(entry), Err(PaymentError::DuplicateReference));
    }
    #[test]
    fn rejects_unbalanced_journal() {
        let entry = JournalEntry {
            reference: "bad".into(),
            postings: vec![
                Posting {
                    account: "a".into(),
                    debit: 100,
                    credit: 0,
                    currency: sek(),
                },
                Posting {
                    account: "b".into(),
                    debit: 0,
                    credit: 99,
                    currency: sek(),
                },
            ],
        };
        assert_eq!(entry.validate(), Err(PaymentError::UnbalancedJournal));
    }
}
