use std::sync::Arc;

#[cfg(feature = "alloc-count")]
#[global_allocator]
static ALLOC: voltrus::alloc_count::Counting = voltrus::alloc_count::Counting;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    voltrus::run(Arc::new(voltrus::edition::Community)).await
}
