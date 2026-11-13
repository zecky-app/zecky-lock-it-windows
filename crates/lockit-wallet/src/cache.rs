//! Compact blocks waiting to be scanned, kept in memory only.
//!
//! The sync downloads a range of compact blocks, scans it into the wallet
//! database and deletes it again, so nothing about the chain is kept on disk
//! beyond what the wallet needs.

use std::{collections::BTreeMap, sync::Mutex};

use async_trait::async_trait;
use zcash_client_backend::{
    data_api::{
        chain::{error, BlockCache, BlockSource},
        scanning::ScanRange,
    },
    proto::compact_formats::CompactBlock,
};
use zcash_protocol::consensus::BlockHeight;

/// The in-memory cache.
#[derive(Default)]
pub(crate) struct MemoryCache {
    blocks: Mutex<BTreeMap<u32, CompactBlock>>,
}

/// The cache cannot fail; it only exists to satisfy the trait's error type.
#[derive(Debug, thiserror::Error)]
#[error("the block cache failed")]
pub(crate) struct CacheError;

fn height_of(block: &CompactBlock) -> u32 {
    block.height as u32
}

impl BlockSource for MemoryCache {
    type Error = CacheError;

    fn with_blocks<F, WalletErrT>(
        &self,
        from_height: Option<BlockHeight>,
        limit: Option<usize>,
        mut with_block: F,
    ) -> Result<(), error::Error<WalletErrT, Self::Error>>
    where
        F: FnMut(CompactBlock) -> Result<(), error::Error<WalletErrT, Self::Error>>,
    {
        let from = from_height.map(u32::from).unwrap_or(0);
        let blocks: Vec<CompactBlock> = {
            let guard = self.blocks.lock().map_err(|_| error::Error::BlockSource(CacheError))?;
            guard.range(from..).take(limit.unwrap_or(usize::MAX)).map(|(_, b)| b.clone()).collect()
        };
        for block in blocks {
            with_block(block)?;
        }
        Ok(())
    }
}

#[async_trait]
impl BlockCache for MemoryCache {
    fn get_tip_height(&self, range: Option<&ScanRange>) -> Result<Option<BlockHeight>, Self::Error> {
        let guard = self.blocks.lock().map_err(|_| CacheError)?;
        let tip = match range {
            Some(range) => {
                let r = range.block_range();
                guard.range(u32::from(r.start)..u32::from(r.end)).next_back().map(|(h, _)| *h)
            }
            None => guard.keys().next_back().copied(),
        };
        Ok(tip.map(BlockHeight::from_u32))
    }

    async fn read(&self, range: &ScanRange) -> Result<Vec<CompactBlock>, Self::Error> {
        let guard = self.blocks.lock().map_err(|_| CacheError)?;
        let r = range.block_range();
        Ok(guard.range(u32::from(r.start)..u32::from(r.end)).map(|(_, b)| b.clone()).collect())
    }

    async fn insert(&self, compact_blocks: Vec<CompactBlock>) -> Result<(), Self::Error> {
        let mut guard = self.blocks.lock().map_err(|_| CacheError)?;
        for block in compact_blocks {
            guard.insert(height_of(&block), block);
        }
        Ok(())
    }

    async fn delete(&self, range: ScanRange) -> Result<(), Self::Error> {
        let mut guard = self.blocks.lock().map_err(|_| CacheError)?;
        let r = range.block_range();
        let doomed: Vec<u32> = guard.range(u32::from(r.start)..u32::from(r.end)).map(|(h, _)| *h).collect();
        for height in doomed {
            guard.remove(&height);
        }
        Ok(())
    }
}
