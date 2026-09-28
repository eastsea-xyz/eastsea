//! A deterministic-runtime context whose disk can be made slow or stalled.
//!
//! Wraps Commonware's deterministic `Context` and delegates everything to it,
//! except that every blob write, resize and sync first sleeps (virtual time)
//! for the validator's current disk delay. The delay is shared through
//! [`Disk`], so a test can slow a validator's disk down, stall it outright,
//! and let it recover while the validator keeps running.

use commonware_runtime::{
    deterministic, signal::Signal, telemetry::metrics::{Metric, Registered}, Blob, BlobVersion, BufferPool,
    BufferPooler, Clock, Error, Handle, IoBufs, IoBufsMut, Metrics, Name, ReadOptions, Spawner, Storage, Supervisor,
    WriteOptions,
};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// The disk delay of one validator, in milliseconds per write, resize or sync.
#[derive(Clone, Default)]
pub struct Disk(Arc<AtomicU64>);

impl Disk {
    pub fn set(&self, delay: Duration) {
        self.0.store(delay.as_millis() as u64, Ordering::SeqCst);
    }

    fn delay(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

pub struct SlowDisk {
    inner: deterministic::Context,
    disk: Disk,
}

impl SlowDisk {
    pub fn new(inner: deterministic::Context, disk: Disk) -> Self {
        Self { inner, disk }
    }
}

impl Supervisor for SlowDisk {
    fn name(&self) -> Name {
        self.inner.name()
    }

    fn child(&self, label: &'static str) -> Self {
        Self { inner: self.inner.child(label), disk: self.disk.clone() }
    }

    fn with_attribute(self, key: &'static str, value: impl std::fmt::Display) -> Self {
        Self { inner: self.inner.with_attribute(key, value), disk: self.disk }
    }
}

impl Spawner for SlowDisk {
    fn shared(self, blocking: bool) -> Self {
        Self { inner: self.inner.shared(blocking), disk: self.disk }
    }

    fn dedicated(self) -> Self {
        Self { inner: self.inner.dedicated(), disk: self.disk }
    }

    fn spawn<F, Fut, T>(self, f: F) -> Handle<T>
    where
        F: FnOnce(Self) -> Fut + Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let disk = self.disk;
        self.inner.spawn(move |inner| f(Self { inner, disk }))
    }

    fn stop(self, value: i32, timeout: Option<Duration>) -> impl Future<Output = Result<(), Error>> + Send {
        self.inner.stop(value, timeout)
    }

    fn stopped(&self) -> Signal {
        self.inner.stopped()
    }
}

impl Metrics for SlowDisk {
    fn register<N: Into<String>, H: Into<String>, M: Metric>(&self, name: N, help: H, metric: M) -> Registered<M> {
        self.inner.register(name, help, metric)
    }

    fn encode(&self) -> String {
        self.inner.encode()
    }
}

impl Clock for SlowDisk {
    fn current(&self) -> SystemTime {
        self.inner.current()
    }

    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send + 'static {
        self.inner.sleep(duration)
    }

    fn sleep_until(&self, deadline: SystemTime) -> impl Future<Output = ()> + Send + 'static {
        self.inner.sleep_until(deadline)
    }
}

impl governor::clock::Clock for SlowDisk {
    type Instant = SystemTime;

    fn now(&self) -> SystemTime {
        self.inner.current()
    }
}

impl governor::clock::ReasonablyRealtime for SlowDisk {}

impl rand::TryRng for SlowDisk {
    type Error = std::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        self.inner.try_next_u32()
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        self.inner.try_next_u64()
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
        self.inner.try_fill_bytes(dest)
    }
}

impl rand::TryCryptoRng for SlowDisk {}

impl BufferPooler for SlowDisk {
    fn network_buffer_pool(&self) -> &BufferPool {
        self.inner.network_buffer_pool()
    }

    fn storage_buffer_pool(&self) -> &BufferPool {
        self.inner.storage_buffer_pool()
    }
}

impl Storage for SlowDisk {
    type Blob = SlowBlob<<deterministic::Context as Storage>::Blob>;

    async fn open_versioned(
        &self,
        partition: &str,
        name: &[u8],
        versions: std::ops::RangeInclusive<BlobVersion>,
    ) -> Result<(Self::Blob, u64, BlobVersion), Error> {
        let (blob, len, version) = self.inner.open_versioned(partition, name, versions).await?;
        Ok((SlowBlob { inner: blob, clock: Arc::new(self.inner.child("disk")), disk: self.disk.clone() }, len, version))
    }

    async fn remove(&self, partition: &str, name: Option<&[u8]>) -> Result<(), Error> {
        self.inner.remove(partition, name).await
    }

    async fn scan(&self, partition: &str) -> Result<Vec<Vec<u8>>, Error> {
        self.inner.scan(partition).await
    }
}

#[derive(Clone)]
pub struct SlowBlob<B> {
    inner: B,
    clock: Arc<deterministic::Context>,
    disk: Disk,
}

impl<B: Blob> SlowBlob<B> {
    async fn pause(&self) {
        let delay = self.disk.delay();
        if !delay.is_zero() {
            self.clock.sleep(delay).await;
        }
    }
}

impl<B: Blob> Blob for SlowBlob<B> {
    fn read_at_buf(
        &self,
        offset: u64,
        len: usize,
        bufs: impl Into<IoBufsMut> + Send,
        options: ReadOptions,
    ) -> impl Future<Output = Result<IoBufsMut, Error>> + Send {
        self.inner.read_at_buf(offset, len, bufs, options)
    }

    fn read_at(&self, offset: u64, len: usize, options: ReadOptions) -> impl Future<Output = Result<IoBufsMut, Error>> + Send {
        self.inner.read_at(offset, len, options)
    }

    async fn write_at(&self, offset: u64, bufs: impl Into<IoBufs> + Send, options: WriteOptions) -> Result<(), Error> {
        self.pause().await;
        self.inner.write_at(offset, bufs, options).await
    }

    async fn resize(&self, len: u64) -> Result<(), Error> {
        self.pause().await;
        self.inner.resize(len).await
    }

    async fn sync(&self) -> Result<(), Error> {
        self.pause().await;
        self.inner.sync().await
    }

    #[allow(clippy::async_yields_async)]
    async fn start_sync(&self) -> Handle<()> {
        self.pause().await;
        self.inner.start_sync().await
    }
}
