//! State management and database authority for pkg (ADR-006).

pub mod db;

pub use db::{
    IntegrationRecord, NewStoreObject, StateDatabase, StoreObjectRecord, TransactionRecord,
};
