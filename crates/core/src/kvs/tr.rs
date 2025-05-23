#[allow(unused_imports, reason = "Not used when none of the storage backends are enabled.")]
use super::api::Transaction;
use super::Key;
use super::KeyEncode;
use super::Val;
use super::Version;
use crate::cf;
use crate::dbs::node::Timestamp;
use crate::doc::CursorValue;
use crate::err::Error;
use crate::idg::u32::U32;
use crate::key::debug::Sprintable;
use crate::kvs::batch::Batch;
use crate::kvs::clock::SizedClock;
#[cfg(any(feature = "kv-tikv", feature = "kv-fdb", feature = "kv-indxdb",))]
use crate::kvs::savepoint::SavePointImpl;
use crate::kvs::stash::Stash;
use crate::kvs::KeyDecode as _;
use crate::sql;
use crate::sql::thing::Thing;
use crate::vs::VersionStamp;
use sql::statements::DefineTableStatement;
use std::fmt;
use std::fmt::Debug;
use std::ops::Range;
use std::sync::Arc;

const TARGET: &str = "surrealdb::core::kvs::tr";

/// Used to determine the behaviour when a transaction is not closed correctly
#[derive(Debug, Default)]
pub enum Check {
	#[default]
	None,
	Warn,
	Error,
}

/// Specifies whether the transaction is read-only or writeable.
#[derive(Copy, Clone)]
pub enum TransactionType {
	Read,
	Write,
}

impl From<bool> for TransactionType {
	fn from(value: bool) -> Self {
		match value {
			true => TransactionType::Write,
			false => TransactionType::Read,
		}
	}
}

/// Specifies whether the transaction is optimistic or pessimistic.
#[derive(Copy, Clone)]
pub enum LockType {
	Pessimistic,
	Optimistic,
}

impl From<bool> for LockType {
	fn from(value: bool) -> Self {
		match value {
			true => LockType::Pessimistic,
			false => LockType::Optimistic,
		}
	}
}

/// A set of undoable updates and requests against a dataset.
#[non_exhaustive]
pub struct Transactor {
	pub(super) inner: Inner,
	pub(super) stash: Stash,
	pub(super) cf: cf::Writer,
	pub(super) clock: Arc<SizedClock>,
}

pub(super) enum Inner {
	#[cfg(feature = "kv-mem")]
	Mem(super::mem::Transaction),
	#[cfg(feature = "kv-rocksdb")]
	RocksDB(super::rocksdb::Transaction),
	#[cfg(feature = "kv-indxdb")]
	IndxDB(super::indxdb::Transaction),
	#[cfg(feature = "kv-tikv")]
	TiKV(super::tikv::Transaction),
	#[cfg(feature = "kv-fdb")]
	FoundationDB(super::fdb::Transaction),
	#[cfg(feature = "kv-surrealkv")]
	SurrealKV(super::surrealkv::Transaction),
}

impl fmt::Display for Transactor {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		#![allow(unused_variables)]
		match &self.inner {
			#[cfg(feature = "kv-mem")]
			Inner::Mem(_) => write!(f, "memory"),
			#[cfg(feature = "kv-rocksdb")]
			Inner::RocksDB(_) => write!(f, "rocksdb"),
			#[cfg(feature = "kv-indxdb")]
			Inner::IndxDB(_) => write!(f, "indxdb"),
			#[cfg(feature = "kv-tikv")]
			Inner::TiKV(_) => write!(f, "tikv"),
			#[cfg(feature = "kv-fdb")]
			Inner::FoundationDB(_) => write!(f, "fdb"),
			#[cfg(feature = "kv-surrealkv")]
			Inner::SurrealKV(_) => write!(f, "surrealkv"),
			#[allow(unreachable_patterns)]
			_ => unreachable!(),
		}
	}
}

macro_rules! expand_inner {
	( $v:expr, $arm:pat_param => $b:block ) => {
		match $v {
			#[cfg(feature = "kv-mem")]
			Inner::Mem($arm) => $b,
			#[cfg(feature = "kv-rocksdb")]
			Inner::RocksDB($arm) => $b,
			#[cfg(feature = "kv-indxdb")]
			Inner::IndxDB($arm) => $b,
			#[cfg(feature = "kv-tikv")]
			Inner::TiKV($arm) => $b,
			#[cfg(feature = "kv-fdb")]
			Inner::FoundationDB($arm) => $b,
			#[cfg(feature = "kv-surrealkv")]
			Inner::SurrealKV($arm) => $b,
			#[allow(unreachable_patterns)]
			_ => unreachable!(),
		}
	};
}

impl Transactor {
	// Allow unused_variables when no storage is enabled as none of the values are used then.
	#![cfg_attr(
		not(any(
			feature = "kv-mem",
			feature = "kv-rocksdb",
			feature = "kv-indxdb",
			feature = "kv-tikv",
			feature = "kv-fdb",
			feature = "kv-surrealkv",
		)),
		allow(unused_variables)
	)]
	// --------------------------------------------------
	// Integral methods
	// --------------------------------------------------

	/// Specify how we should handle unclosed transactions.
	///
	/// If a transaction is not cancelled or rolled back then
	/// this can cause issues on some storage engine
	/// implementations. In tests we can ignore unhandled
	/// transactions, whilst in development we should panic
	/// so that any unintended behaviour is detected, and in
	/// production we should only log a warning.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub(crate) fn check_level(&mut self, check: Check) {
		expand_inner!(&mut self.inner, v => { v.check_level(check) })
	}

	/// Check if transaction is finished.
	///
	/// If the transaction has been cancelled or committed,
	/// then this function will return [`true`], and any further
	/// calls to functions on this transaction will result
	/// in a [`Error::TxFinished`] error.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn closed(&self) -> bool {
		trace!(target: TARGET, "Closed");
		expand_inner!(&self.inner, v => { v.closed() })
	}

	/// Cancel a transaction.
	///
	/// This reverses all changes made within the transaction.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn cancel(&mut self) -> Result<(), Error> {
		trace!(target: TARGET, "Cancel");
		expand_inner!(&mut self.inner, v => { v.cancel().await })
	}

	/// Commit a transaction.
	///
	/// This attempts to commit all changes made within the transaction.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn commit(&mut self) -> Result<(), Error> {
		trace!(target: TARGET, "Commit");
		expand_inner!(&mut self.inner, v => { v.commit().await })
	}

	/// Check if a key exists in the datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn exists<K>(&mut self, key: K, version: Option<u64>) -> Result<bool, Error>
	where
		K: KeyEncode + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), version = version, "Exists");
		expand_inner!(&mut self.inner, v => { v.exists(key, version).await })
	}

	/// Fetch a key from the datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn get<K>(&mut self, key: K, version: Option<u64>) -> Result<Option<Val>, Error>
	where
		K: KeyEncode + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), version = version, "Get");
		expand_inner!(&mut self.inner, v => { v.get(key, version).await })
	}

	/// Fetch many keys from the datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn getm<K>(&mut self, keys: Vec<K>) -> Result<Vec<Option<Val>>, Error>
	where
		K: KeyEncode + Debug,
	{
		let mut keys_encoded = Vec::new();
		for k in keys {
			keys_encoded.push(k.encode_owned()?);
		}
		trace!(target: TARGET, keys = keys_encoded.sprint(), "GetM");
		expand_inner!(&mut self.inner, v => { v.getm(keys_encoded).await })
	}

	/// Retrieve a specific range of keys from the datastore.
	///
	/// This function fetches all matching key-value pairs from the underlying datastore in grouped batches.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn getr<K>(
		&mut self,
		rng: Range<K>,
		version: Option<u64>,
	) -> Result<Vec<(Key, Val)>, Error>
	where
		K: KeyEncode + Debug,
	{
		let beg: Key = rng.start.encode_owned()?;
		let end: Key = rng.end.encode_owned()?;
		let rng = beg.as_slice()..end.as_slice();
		trace!(target: TARGET, rng = rng.sprint(), version = version, "GetR");
		expand_inner!(&mut self.inner, v => { v.getr(beg..end, version).await })
	}

	/// Retrieve a specific prefixed range of keys from the datastore.
	///
	/// This function fetches all matching key-value pairs from the underlying datastore in grouped batches.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn getp<K>(&mut self, key: K) -> Result<Vec<(Key, Val)>, Error>
	where
		K: KeyEncode + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), "GetP");
		expand_inner!(&mut self.inner, v => { v.getp(key).await })
	}

	/// Insert or update a key in the datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn set<K, V>(&mut self, key: K, val: V, version: Option<u64>) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
		V: Into<Val> + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), version = version, "Set");
		expand_inner!(&mut self.inner, v => { v.set(key, val, version).await })
	}

	/// Insert or replace a key in the datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn replace<K, V>(&mut self, key: K, val: V) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
		V: Into<Val> + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), "Replace");
		expand_inner!(&mut self.inner, v => { v.replace(key, val).await })
	}

	/// Insert a key if it doesn't exist in the datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn put<K, V>(&mut self, key: K, val: V, version: Option<u64>) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
		V: Into<Val> + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), version = version, "Put");
		expand_inner!(&mut self.inner, v => { v.put(key, val, version).await })
	}

	/// Update a key in the datastore if the current value matches a condition.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn putc<K, V>(&mut self, key: K, val: V, chk: Option<V>) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
		V: Into<Val> + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), "PutC");
		expand_inner!(&mut self.inner, v => { v.putc(key, val, chk).await })
	}

	/// Delete a key from the datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn del<K>(&mut self, key: K) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), "Del");
		expand_inner!(&mut self.inner, v => { v.del(key).await })
	}

	/// Delete a key from the datastore if the current value matches a condition.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn delc<K, V>(&mut self, key: K, chk: Option<V>) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
		V: Into<Val> + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), "DelC");
		expand_inner!(&mut self.inner, v => { v.delc(key, chk).await })
	}

	/// Delete a range of keys from the datastore.
	///
	/// This function deletes all matching key-value pairs from the underlying datastore in grouped batches.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn delr<K>(&mut self, rng: Range<K>) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
	{
		let beg: Key = rng.start.encode_owned()?;
		let end: Key = rng.end.encode_owned()?;
		let rng = beg.as_slice()..end.as_slice();
		trace!(target: TARGET, rng = rng.sprint(), "DelR");
		expand_inner!(&mut self.inner, v => { v.delr(beg..end).await })
	}

	/// Delete a prefixed range of keys from the datastore.
	///
	/// This function deletes all matching key-value pairs from the underlying datastore in grouped batches.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn delp<K>(&mut self, key: K) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), "DelP");
		expand_inner!(&mut self.inner, v => { v.delp(key).await })
	}

	/// Delete all versions of a key from the datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn clr<K>(&mut self, key: K) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), "Clr");
		expand_inner!(&mut self.inner, v => { v.clr(key).await })
	}

	/// Delete all versions of a key from the datastore if the current value matches a condition.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn clrc<K, V>(&mut self, key: K, chk: Option<V>) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
		V: Into<Val> + Debug,
	{
		let key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), "ClrC");
		expand_inner!(&mut self.inner, v => { v.clrc(key, chk).await })
	}

	/// Delete all versions of a range of keys from the datastore.
	///
	/// This function deletes all matching key-value pairs from the underlying datastore in grouped batches.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn clrr<K>(&mut self, rng: Range<K>) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
	{
		let beg: Key = rng.start.encode_owned()?;
		let end: Key = rng.end.encode_owned()?;
		let rng = beg.as_slice()..end.as_slice();
		trace!(target: TARGET, rng = rng.sprint(), "ClrR");
		expand_inner!(&mut self.inner, v => { v.clrr(beg..end).await })
	}

	/// Delete all versions of a prefixed range of keys from the datastore.
	///
	/// This function deletes all matching key-value pairs from the underlying datastore in grouped batches.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn clrp<K>(&mut self, key: K) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
	{
		let key: Key = key.encode_owned()?;
		trace!(target: TARGET, key = key.sprint(), "ClrP");
		expand_inner!(&mut self.inner, v => { v.clrp(key).await })
	}

	/// Retrieve a specific range of keys from the datastore.
	///
	/// This function fetches the full range of keys without values, in a single request to the underlying datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn keys<K>(
		&mut self,
		rng: Range<K>,
		limit: u32,
		version: Option<u64>,
	) -> Result<Vec<Key>, Error>
	where
		K: KeyEncode + Debug,
	{
		let beg: Key = rng.start.encode_owned()?;
		let end: Key = rng.end.encode_owned()?;
		let rng = beg.as_slice()..end.as_slice();
		trace!(target: TARGET, rng = rng.sprint(), limit = limit, version = version, "Keys");
		if beg > end {
			return Ok(vec![]);
		}
		expand_inner!(&mut self.inner, v => { v.keys(beg..end, limit, version).await })
	}

	/// Retrieve a specific range of keys from the datastore.
	///
	/// This function fetches the full range of keys without values, in a single request to the underlying datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn keysr<K>(
		&mut self,
		rng: Range<K>,
		limit: u32,
		version: Option<u64>,
	) -> Result<Vec<Key>, Error>
	where
		K: KeyEncode + Debug,
	{
		let beg: Key = rng.start.encode_owned()?;
		let end: Key = rng.end.encode_owned()?;
		let rng = beg.as_slice()..end.as_slice();
		trace!(target: TARGET, rng = rng.sprint(), limit = limit, version = version, "Keysr");
		if beg > end {
			return Ok(vec![]);
		}
		expand_inner!(&mut self.inner, v => { v.keysr(beg..end, limit, version).await })
	}

	/// Retrieve a specific range of keys from the datastore.
	///
	/// This function fetches the full range of key-value pairs, in a single request to the underlying datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn scan<K>(
		&mut self,
		rng: Range<K>,
		limit: u32,
		version: Option<u64>,
	) -> Result<Vec<(Key, Val)>, Error>
	where
		K: KeyEncode + Debug,
	{
		let beg: Key = rng.start.encode_owned()?;
		let end: Key = rng.end.encode_owned()?;
		let rng = beg.as_slice()..end.as_slice();
		trace!(target: TARGET, rng = rng.sprint(), limit = limit, version = version, "Scan");
		if beg > end {
			return Ok(vec![]);
		}
		expand_inner!(&mut self.inner, v => { v.scan(beg..end, limit, version).await })
	}

	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn scanr<K>(
		&mut self,
		rng: Range<K>,
		limit: u32,
		version: Option<u64>,
	) -> Result<Vec<(Key, Val)>, Error>
	where
		K: Into<Key> + Debug,
	{
		let beg: Key = rng.start.into();
		let end: Key = rng.end.into();
		let rng = beg.as_slice()..end.as_slice();
		trace!(target: TARGET, rng = rng.sprint(), limit = limit, version = version, "Scanr");
		if beg > end {
			return Ok(vec![]);
		}
		expand_inner!(&mut self.inner, v => { v.scanr(beg..end, limit, version).await })
	}

	/// Retrieve a batched scan over a specific range of keys in the datastore.
	///
	/// This function fetches keys, in batches, with multiple requests to the underlying datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn batch_keys<K>(
		&mut self,
		rng: Range<K>,
		batch: u32,
		version: Option<u64>,
	) -> Result<Batch<Key>, Error>
	where
		K: KeyEncode + Debug,
	{
		let beg: Key = rng.start.encode_owned()?;
		let end: Key = rng.end.encode_owned()?;
		let rng = beg.as_slice()..end.as_slice();
		trace!(target: TARGET, rng = rng.sprint(), version = version, "Batch");
		expand_inner!(&mut self.inner, v => { v.batch_keys(beg..end, batch, version).await })
	}

	/// Count the total number of keys within a range in the datastore.
	///
	/// This function fetches the total count, in batches, with multiple requests to the underlying datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn count<K>(&mut self, rng: Range<K>) -> Result<usize, Error>
	where
		K: KeyEncode + Debug,
	{
		let beg: Key = rng.start.encode_owned()?;
		let end: Key = rng.end.encode_owned()?;
		let rng = beg.as_slice()..end.as_slice();
		trace!(target: TARGET, rng = rng.sprint(), "Count");
		expand_inner!(&mut self.inner, v => { v.count(beg..end).await })
	}

	/// Retrieve a batched scan over a specific range of keys in the datastore.
	///
	/// This function fetches key-value pairs, in batches, with multiple requests to the underlying datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn batch_keys_vals<K>(
		&mut self,
		rng: Range<K>,
		batch: u32,
		version: Option<u64>,
	) -> Result<Batch<(Key, Val)>, Error>
	where
		K: KeyEncode + Debug,
	{
		let beg: Key = rng.start.encode_owned()?;
		let end: Key = rng.end.encode_owned()?;
		let rng = beg.as_slice()..end.as_slice();
		trace!(target: TARGET, rng = rng.sprint(), version = version, "Batch");
		expand_inner!(&mut self.inner, v => { v.batch_keys_vals(beg..end, batch, version).await })
	}

	/// Retrieve a batched scan of all versions over a specific range of keys in the datastore.
	///
	/// This function fetches key-value-version pairs, in batches, with multiple requests to the underlying datastore.
	#[instrument(level = "trace", target = "surrealdb::core::kvs::tr", skip_all)]
	pub async fn batch_keys_vals_versions<K>(
		&mut self,
		rng: Range<K>,
		batch: u32,
	) -> Result<Batch<(Key, Val, Version, bool)>, Error>
	where
		K: KeyEncode + Debug,
	{
		let beg: Key = rng.start.encode_owned()?;
		let end: Key = rng.end.encode_owned()?;
		let rng = beg.as_slice()..end.as_slice();
		trace!(target: TARGET, rng = rng.sprint(), "BatchVersions");
		expand_inner!(&mut self.inner, v => { v.batch_keys_vals_versions(beg..end, batch).await })
	}

	/// Obtain a new change timestamp for a key
	/// which is replaced with the current timestamp when the transaction is committed.
	/// NOTE: This should be called when composing the change feed entries for this transaction,
	/// which should be done immediately before the transaction commit.
	/// That is to keep other transactions commit delay(pessimistic) or conflict(optimistic) as less as possible.
	pub async fn get_timestamp<K>(&mut self, key: K) -> Result<VersionStamp, Error>
	where
		K: KeyEncode + Debug,
	{
		let key = key.encode_owned()?;
		expand_inner!(&mut self.inner, v => { v.get_timestamp(key).await })
	}

	/// Insert or update a key in the datastore.
	pub async fn set_versionstamped<K, V>(
		&mut self,
		ts_key: K,
		prefix: K,
		suffix: K,
		val: V,
	) -> Result<(), Error>
	where
		K: KeyEncode + Debug,
		V: Into<Val> + Debug,
	{
		let ts_key = ts_key.encode_owned()?;
		let prefix = prefix.encode_owned()?;
		let suffix = suffix.encode_owned()?;
		expand_inner!(&mut self.inner, v => { v.set_versionstamp(ts_key, prefix, suffix, val).await })
	}

	// --------------------------------------------------
	// Additional methods
	// --------------------------------------------------

	/// Clock retrieves the current timestamp, without guaranteeing
	/// monotonicity in all implementations.
	///
	/// It is used for unreliable ordering of events as well as
	/// handling of timeouts. Operations that are not guaranteed to be correct.
	/// But also allows for lexicographical ordering.
	///
	/// Public for tests, but not required for usage from a user perspective.
	pub async fn clock(&self) -> Timestamp {
		self.clock.now().await
	}

	// change will record the change in the changefeed if enabled.
	// To actually persist the record changes into the underlying kvs,
	// you must call the `complete_changes` function and then commit the transaction.
	#[expect(clippy::too_many_arguments)]
	pub(crate) fn record_change(
		&mut self,
		ns: &str,
		db: &str,
		tb: &str,
		id: &Thing,
		previous: CursorValue,
		current: CursorValue,
		store_difference: bool,
	) {
		self.cf.record_cf_change(ns, db, tb, id.clone(), previous, current, store_difference)
	}

	// Records the table (re)definition in the changefeed if enabled.
	pub(crate) fn record_table_change(
		&mut self,
		ns: &str,
		db: &str,
		tb: &str,
		dt: &DefineTableStatement,
	) {
		self.cf.define_table(ns, db, tb, dt)
	}

	pub(crate) async fn get_idg(&mut self, key: &Key) -> Result<U32, Error> {
		Ok(if let Some(v) = self.stash.get(key) {
			v
		} else {
			let val = self.get(key.clone(), None).await?;
			if let Some(val) = val {
				U32::new(key.clone(), Some(val)).await?
			} else {
				U32::new(key.clone(), None).await?
			}
		})
	}

	/// Gets the next namespace id
	pub(crate) async fn get_next_ns_id(&mut self) -> Result<u32, Error> {
		let key = crate::key::root::ni::Ni::default().encode_owned()?;
		let mut seq = self.get_idg(&key).await?;
		let nid = seq.get_next_id();
		self.stash.set(key, seq.clone());
		let (k, v) = seq.finish().unwrap();
		self.replace(k, v).await?;
		Ok(nid)
	}

	/// Gets the next database id for the given namespace
	pub(crate) async fn get_next_db_id(&mut self, ns: u32) -> Result<u32, Error> {
		let key = crate::key::namespace::di::new(ns).encode_owned()?;
		let mut seq = self.get_idg(&key).await?;
		let nid = seq.get_next_id();
		self.stash.set(key, seq.clone());
		let (k, v) = seq.finish().unwrap();
		self.replace(k, v).await?;
		Ok(nid)
	}

	/// Gets the next table id for the given namespace and database
	pub(crate) async fn get_next_tb_id(&mut self, ns: u32, db: u32) -> Result<u32, Error> {
		let key = crate::key::database::ti::new(ns, db).encode_owned()?;
		let mut seq = self.get_idg(&key).await?;
		let nid = seq.get_next_id();
		self.stash.set(key, seq.clone());
		let (k, v) = seq.finish().unwrap();
		self.replace(k, v).await?;
		Ok(nid)
	}

	/// Removes the given namespace from the sequence.
	#[expect(unused)]
	pub(crate) async fn remove_ns_id(&mut self, ns: u32) -> Result<(), Error> {
		let key = crate::key::root::ni::Ni::default().encode_owned()?;
		let mut seq = self.get_idg(&key).await?;
		seq.remove_id(ns);
		self.stash.set(key, seq.clone());
		let (k, v) = seq.finish().unwrap();
		self.replace(k, v).await?;
		Ok(())
	}

	/// Removes the given database from the sequence.
	#[expect(unused)]
	pub(crate) async fn remove_db_id(&mut self, ns: u32, db: u32) -> Result<(), Error> {
		let key = crate::key::namespace::di::new(ns).encode_owned()?;
		let mut seq = self.get_idg(&key).await?;
		seq.remove_id(db);
		self.stash.set(key, seq.clone());
		let (k, v) = seq.finish().unwrap();
		self.replace(k, v).await?;
		Ok(())
	}

	/// Removes the given table from the sequence.
	#[expect(unused)]
	pub(crate) async fn remove_tb_id(&mut self, ns: u32, db: u32, tb: u32) -> Result<(), Error> {
		let key = crate::key::database::ti::new(ns, db).encode_owned()?;
		let mut seq = self.get_idg(&key).await?;
		seq.remove_id(tb);
		self.stash.set(key, seq.clone());
		let (k, v) = seq.finish().unwrap();
		self.replace(k, v).await?;
		Ok(())
	}

	// complete_changes will complete the changefeed recording for the given namespace and database.
	//
	// Under the hood, this function calls the transaction's `set_versionstamped_key` for each change.
	// Every change must be recorded by calling this struct's `record_change` function beforehand.
	// If there were no preceding `record_change` function calls for this transaction, this function will do nothing.
	//
	// This function should be called only after all the changes have been made to the transaction.
	// Otherwise, changes are missed in the change feed.
	//
	// This function should be called immediately before calling the commit function to guarantee that
	// the lock, if needed by lock=true, is held only for the duration of the commit, not the entire transaction.
	//
	// This function is here because it needs access to mutably borrow the transaction.
	//
	// Lastly, you should set lock=true if you want the changefeed to be correctly ordered for
	// non-FDB backends.
	pub(crate) async fn complete_changes(&mut self, _lock: bool) -> Result<(), Error> {
		let changes = self.cf.get()?;
		for (tskey, prefix, suffix, v) in changes {
			self.set_versionstamped(tskey, prefix, suffix, v).await?
		}
		Ok(())
	}

	// set_timestamp_for_versionstamp correlates the given timestamp with the current versionstamp.
	// This allows get_versionstamp_from_timestamp to obtain the versionstamp from the timestamp later.
	pub(crate) async fn set_timestamp_for_versionstamp(
		&mut self,
		ts: u64,
		ns: &str,
		db: &str,
	) -> Result<VersionStamp, Error> {
		// This also works as an advisory lock on the ts keys so that there is
		// on other concurrent transactions that can write to the ts_key or the keys after it.
		let key = crate::key::database::vs::new(ns, db);
		let vst = self.get_timestamp(key).await?;
		trace!(
			target: TARGET,
			"Setting timestamp {} for versionstamp {:?} in ns: {}, db: {}",
			ts,
			vst.into_u64_lossy(),
			ns,
			db
		);

		// Ensure there are no keys after the ts_key
		// Otherwise we can go back in time!
		let mut ts_key = crate::key::database::ts::new(ns, db, ts);
		let begin = ts_key.encode()?;
		let end = crate::key::database::ts::suffix(ns, db)?;
		let ts_pairs: Vec<(Vec<u8>, Vec<u8>)> = self.getr(begin..end, None).await?;
		let latest_ts_pair = ts_pairs.last();
		if let Some((k, _)) = latest_ts_pair {
			trace!(
				target: TARGET,
				"There already was a greater committed timestamp {} in ns: {}, db: {} found: {}",
				ts,
				ns,
				db,
				k.sprint()
			);
			let k = crate::key::database::ts::Ts::decode(k)?;
			let latest_ts = k.ts;
			if latest_ts >= ts {
				warn!("ts {ts} is less than the latest ts {latest_ts}");
				ts_key = crate::key::database::ts::new(ns, db, latest_ts + 1);
			}
		}
		self.replace(ts_key, vst.as_bytes()).await?;
		Ok(vst)
	}

	pub(crate) async fn get_versionstamp_from_timestamp(
		&mut self,
		ts: u64,
		ns: &str,
		db: &str,
	) -> Result<Option<VersionStamp>, Error> {
		let start = crate::key::database::ts::prefix(ns, db)?;
		let ts_key = crate::key::database::ts::new(ns, db, ts + 1).encode_owned()?;
		let end = ts_key.encode_owned()?;
		let ts_pairs = self.getr(start..end, None).await?;
		let latest_ts_pair = ts_pairs.last();
		if let Some((_, v)) = latest_ts_pair {
			return Ok(Some(VersionStamp::from_slice(v)?));
		}
		Ok(None)
	}

	pub(crate) async fn new_save_point(&mut self) {
		expand_inner!(&mut self.inner, v => { v.new_save_point() })
	}

	pub(crate) async fn rollback_to_save_point(&mut self) -> Result<(), Error> {
		expand_inner!(&mut self.inner, v => { v.rollback_to_save_point().await })
	}

	pub(crate) async fn release_last_save_point(&mut self) -> Result<(), Error> {
		expand_inner!(&mut self.inner, v => { v.release_last_save_point() })
	}
}
