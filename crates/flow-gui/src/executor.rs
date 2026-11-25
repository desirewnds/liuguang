// SPDX-License-Identifier: Apache-2.0
//! The UI needs a small async executor, independent of the machine's CPU count.
pub struct SmallExecutor(tokio::runtime::Runtime);

impl iced::Executor for SmallExecutor {
    fn new() -> Result<Self, std::io::Error> {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(8)
            .thread_name("flow-runtime")
            .enable_all()
            .build()
            .map(Self)
    }

    fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        drop(self.0.spawn(future));
    }

    fn enter<R>(&self, run: impl FnOnce() -> R) -> R {
        let _guard = self.0.enter();
        run()
    }

    fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        self.0.block_on(future)
    }
}
