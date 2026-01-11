//! OpenAPI documentation configuration.

use utoipa::OpenApi;

use crate::error::ErrorResponse;
use crate::handlers::{
    blockchain, blocks, emission, info, mining, node, peers, scan, script, transactions, utils,
    utxo, wallet,
};

/// Main OpenAPI documentation structure.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Ergo Node API",
        description = "REST API for the Ergo blockchain node. Compatible with Scala node API version 5.0.0.",
        version = "5.0.0",
        license(name = "Apache-2.0", url = "https://www.apache.org/licenses/LICENSE-2.0"),
        contact(
            name = "Ergo Platform",
            url = "https://ergoplatform.org"
        )
    ),
    servers(
        (url = "http://127.0.0.1:9053", description = "Local node")
    ),
    tags(
        (name = "info", description = "Node information and status"),
        (name = "blocks", description = "Block queries and retrieval"),
        (name = "transactions", description = "Transaction submission and queries"),
        (name = "utxo", description = "UTXO (unspent transaction output) queries"),
        (name = "peers", description = "Peer management and discovery"),
        (name = "mining", description = "Mining interface for external miners"),
        (name = "wallet", description = "HD wallet operations"),
        (name = "utils", description = "Utility endpoints"),
        (name = "emission", description = "Emission schedule information"),
        (name = "script", description = "Script compilation and execution"),
        (name = "node", description = "Node management"),
        (name = "scan", description = "External scans (EIP-0001)"),
        (name = "blockchain", description = "Blockchain indexer queries")
    ),
    paths(
        // Info
        info::get_info,
        // Blocks
        blocks::get_blocks,
        blocks::get_block_by_id,
        blocks::get_block_header,
        blocks::get_block_transactions,
        blocks::get_block_at_height,
        // Transactions
        transactions::submit_transaction,
        transactions::check_transaction,
        transactions::get_fee_histogram,
        transactions::get_recommended_fee,
        transactions::get_wait_time,
        transactions::get_transaction,
        transactions::get_unconfirmed,
        transactions::get_unconfirmed_paged,
        transactions::get_unconfirmed_by_id,
        transactions::get_unconfirmed_by_output_id,
        transactions::get_unconfirmed_by_token_id,
        transactions::get_unconfirmed_by_ergo_tree,
        // UTXO
        utxo::get_box_by_id,
        utxo::get_box_binary_by_id,
        utxo::get_boxes_by_address,
        utxo::get_box_with_pool,
        utxo::get_boxes_with_pool,
        utxo::get_genesis_boxes,
        utxo::get_snapshots_info,
        utxo::get_boxes_binary_proof,
        // Peers
        peers::get_all_peers,
        peers::get_connected_peers,
        peers::get_blacklisted,
        peers::connect_peer,
        // Mining
        mining::get_candidate,
        mining::get_candidate_extended,
        mining::submit_solution,
        mining::get_reward_address,
        mining::set_reward_address,
        // Wallet
        wallet::init_wallet,
        wallet::unlock_wallet,
        wallet::lock_wallet,
        wallet::get_status,
        wallet::get_balances,
        wallet::get_addresses,
        wallet::derive_address,
        wallet::get_unspent_boxes,
        wallet::send_transaction,
        // Utils
        utils::get_seed,
        utils::get_seed_with_length,
        utils::hash_blake2b,
        utils::validate_address,
        utils::validate_address_post,
        utils::address_to_raw,
        utils::raw_to_address,
        utils::ergo_tree_to_address,
        utils::ergo_tree_to_address_post,
        // Emission
        emission::get_emission_at_height,
        emission::get_emission_scripts,
        // Script
        script::compile_to_p2s,
        script::compile_to_p2sh,
        script::address_to_tree,
        script::address_to_bytes,
        script::execute_with_context,
        // Node
        node::shutdown,
        // Scan
        scan::register,
        scan::deregister,
        scan::list_all,
        scan::unspent_boxes,
        scan::spent_boxes,
        scan::stop_tracking,
        scan::add_box,
        scan::p2s_rule,
        // Blockchain
        blockchain::get_indexed_height,
        blockchain::get_transaction_by_id,
        blockchain::get_transaction_by_index,
        blockchain::get_box_by_id,
        blockchain::get_box_by_index,
        blockchain::get_token_by_id,
        blockchain::get_balance_by_address,
        blockchain::get_boxes_by_address,
        blockchain::get_boxes_by_token_id,
    ),
    components(
        schemas(
            // Error
            ErrorResponse,
            // Info
            info::NodeInfo,
            // Blocks
            blocks::BlockSummary,
            blocks::BlockHeader,
            blocks::BlockTransactions,
            // Transactions
            transactions::SubmitTx,
            transactions::TxResponse,
            transactions::CheckTxResponse,
            transactions::UnconfirmedTx,
            transactions::FeeHistogramEntry,
            transactions::RecommendedFee,
            transactions::WaitTimeResponse,
            // UTXO
            utxo::BoxResponse,
            utxo::BoxBinaryResponse,
            utxo::TokenAmount,
            utxo::BoxIdsRequest,
            utxo::SnapshotsInfoResponse,
            utxo::GenesisBoxResponse,
            utxo::BinaryProofRequest,
            utxo::BinaryProofResponse,
            // Peers
            peers::PeerInfo,
            peers::ConnectRequest,
            // Mining
            mining::MiningCandidate,
            mining::MiningCandidateExtended,
            mining::SolutionSubmission,
            mining::SolutionResponse,
            mining::RewardAddressResponse,
            mining::RewardAddressRequest,
            // Wallet
            wallet::InitWallet,
            wallet::InitWalletResponse,
            wallet::UnlockWallet,
            wallet::WalletStatus,
            wallet::BalanceResponse,
            wallet::TokenBalance,
            wallet::WalletAddressResponse,
            wallet::SendRequest,
            wallet::WalletTransactionResponse,
            wallet::DeriveAddressRequest,
            wallet::WalletBox,
            // Utils
            utils::SeedResponse,
            utils::HashRequest,
            utils::HashResponse,
            utils::AddressValidation,
            utils::RawAddressResponse,
            utils::ErgoTreeRequest,
            utils::AddressResult,
            // Emission
            emission::EmissionInfo,
            emission::EmissionScripts,
            // Script
            script::CompileRequest,
            script::P2SAddressResponse,
            script::AddressToTreeRequest,
            script::ErgoTreeResponse,
            script::AddressToBytesRequest,
            script::BytesResponse,
            script::ExecuteRequest,
            script::ExecuteResponse,
            // Node
            node::ShutdownResponse,
            // Scan
            scan::ScanIdWrapper,
            scan::ScanIdBoxId,
            scan::BoxWithScanIds,
            scan::ScanBoxResponse,
            scan::ScanResponse,
            scan::P2SAddressRequest,
            scan::ScanRegistrationRequest,
            // Blockchain
            blockchain::IndexedHeightResponse,
            blockchain::IndexedTransactionResponse,
            blockchain::IndexedBoxResponse,
            blockchain::BoxTokenAmount,
            blockchain::TokenInfoResponse,
            blockchain::AddressBalanceResponse,
            blockchain::IndexedTokenBalance,
            blockchain::BlockchainErrorResponse,
            blockchain::SortDirection,
            blockchain::PaginatedBoxResponse,
        )
    )
)]
pub struct ApiDoc;
