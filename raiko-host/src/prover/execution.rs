use std::time::Instant;

use alloy_primitives::FixedBytes;
use ethers_core::types::H160;
use tracing::{info, warn};
use zeth_lib::{builder::{BlockBuilderStrategy, TaikoStrategy}, consts::TKO_MAINNET_CHAIN_SPEC, input::{GuestInput, GuestOutput, TaikoSystemInfo, TaikoProverData},
    host::host::{HostArgs, taiko_run_preflight}, EthereumTxEssence
};
use zeth_lib::protocol_instance::assemble_protocol_instance;
use zeth_lib::protocol_instance::EvidenceType;
use zeth_primitives::{keccak, Address, B256};
use crate::metrics::{inc_sgx_success, observe_input, observe_sgx_gen};

use super::{
    context::Context,
    error::Result,
    proof::{cache::Cache, sgx::execute_sgx},
    request::{ProofInstance, ProofRequest, ProofResponse},
    utils::cache_file_path,
};
use super::proof::succinct::execute_sp1;
use super::proof::powdr::execute_powdr;
use super::proof::risc0::execute_risc0;


pub async fn execute(
    _cache: &Cache,
    ctx: &mut Context,
    req: &ProofRequest,
) -> Result<ProofResponse> {

    // ctx.update_cache_path(req.block);
    // try remove cache file anyway to avoid reorg error
    // because tokio::fs::remove_file haven't guarantee of execution. So, we need to remove
    // twice
    // > Runs the provided function on an executor dedicated to blocking operations.
    // > Tasks will be scheduled as non-mandatory, meaning they may not get executed
    // > in case of runtime shutdown.
    // ctx.remove_cache_file().await?;
    let result = async {
        // 1. load input data into cache path
        let start = Instant::now();
        let input = prepare_input(ctx, req.clone()).await?;
        let elapsed = Instant::now().duration_since(start).as_millis() as i64;
        observe_input(elapsed);
        // 2. pre-build the block
        let build_result = TaikoStrategy::build_from(&TKO_MAINNET_CHAIN_SPEC.clone(), input.clone());
        // TODO: cherry-pick risc0 latest output
        let output = match &build_result {
            Ok((header, mpt_node)) => {
                info!("Verifying final state using provider data ...");
                info!("Final block hash derived successfully. {}", header.hash());
                info!("Final block hash derived successfully. {:?}", header);
                let pi = assemble_protocol_instance(&input, &header)?
                    .instance_hash(req.proof_instance.clone().into());
                GuestOutput::Success((header.clone(), pi))
            }
            Err(_) => {
                warn!("Proving bad block construction!");
                GuestOutput::Failure
            }
        };
        let elapsed = Instant::now().duration_since(start).as_millis() as i64;
        observe_input(elapsed);
        // 3. run proof
        // prune_old_caches(&ctx.cache_path, ctx.max_caches);
        match &req.proof_instance {
            ProofInstance::Sgx => {
                let start = Instant::now();
                let bid = req.block_number;
                let resp = execute_sgx(ctx, req).await?;
                let time_elapsed = Instant::now().duration_since(start).as_millis() as i64;
                observe_sgx_gen(bid, time_elapsed);
                inc_sgx_success(bid);
                Ok(ProofResponse::Sgx(resp))
            }
            ProofInstance::Powdr => {
                let start = Instant::now();
                let bid = req.block_number;
                let resp = execute_powdr().await?;
                let time_elapsed = Instant::now().duration_since(start).as_millis() as i64;
                todo!()
            }
            ProofInstance::PseZk => todo!(),
            ProofInstance::Succinct => {
                let start = Instant::now();
                let bid = req.block_number;
                let resp = execute_sp1(ctx, req).await?;
                let time_elapsed = Instant::now().duration_since(start).as_millis() as i64;
                Ok(ProofResponse::SP1(resp))
            }
            ProofInstance::Risc0(instance) => {
                execute_risc0(input, output, ctx, instance).await?;
                todo!()
            },
            ProofInstance::Native => {
                Ok(ProofResponse::Native(output))
            },
        }
    }
    .await;
    ctx.remove_cache_file().await?;
    result
}

/// prepare input data for guests
pub async fn prepare_input(
    ctx: &mut Context,
    req: ProofRequest,
) -> Result<GuestInput<EthereumTxEssence>> {
    // Todo(Cecilia): should contract address as args, curently hardcode
    let l1_cache = ctx.l1_cache_file.clone();
    let l2_cache = ctx.l2_cache_file.clone();
    tokio::task::spawn_blocking(move || {
        taiko_run_preflight(
            Some(req.l1_rpc),
            TKO_MAINNET_CHAIN_SPEC.clone(),
            Some(req.l2_rpc),
            req.block_number,
            &req.l2_contracts,
            TaikoProverData {
                graffiti: req.graffiti,
                prover: req.prover,
            },
        ).expect("Init taiko failed")
    })
    .await
    .map_err(Into::<super::error::Error>::into)
}

impl From<ProofInstance> for EvidenceType {
    fn from(value: ProofInstance) -> Self {
        match value {
            ProofInstance::Succinct => EvidenceType::Succinct,
            ProofInstance::PseZk => EvidenceType::PseZk,
            ProofInstance::Powdr => EvidenceType::Powdr,
            ProofInstance::Sgx => EvidenceType::Sgx{
                new_pubkey: Address::default()
            },
            ProofInstance::Risc0(_) => EvidenceType::Risc0,
            ProofInstance::Native => EvidenceType::Native,
        }
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_async_block() {
        let result = async { Result::<(), &'static str>::Err("error") };
        println!("must here");
        assert!(result.await.is_err());
    }
}
