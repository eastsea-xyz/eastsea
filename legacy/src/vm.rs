use crate::types::{Address, ContractInfo};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub struct VmExecutionResult {
    pub contract_slot_writes: HashMap<u64, u64>,
    pub account_balance_deltas: Vec<(Address, i64)>,
    pub logs: Vec<String>,
    pub gas_used: u64,
}

pub struct SmartContractEngine;

fn integer_sqrt(n: u64) -> u64 {
    if n < 2 { return n; }
    let mut x0 = n / 2;
    let mut x1 = (x0 + n / x0) / 2;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + n / x0) / 2;
    }
    x0
}

impl SmartContractEngine {
    pub fn derive_contract_address(creator: &Address, nonce: u64, name: &str) -> Address {
        let mut hasher = Sha256::new();
        hasher.update(&creator.0);
        hasher.update(&nonce.to_be_bytes());
        hasher.update(name.as_bytes());
        let hash = hasher.finalize();
        let mut bytes = [0u8; 20];
        bytes.copy_from_slice(&hash[12..32]);
        Address(bytes)
    }

    pub fn deploy(
        creator: Address,
        nonce: u64,
        round: u64,
        name: &str,
        template: &str,
        params: &[u64],
    ) -> (ContractInfo, HashMap<u64, u64>) {
        let address = Self::derive_contract_address(&creator, nonce, name);
        let mut initial_slots = HashMap::new();

        match template.to_lowercase().as_str() {
            "counter" => {
                let init_val = params.get(0).copied().unwrap_or(0);
                initial_slots.insert(0, init_val); // Slot 0: Counter value
            }
            "token" => {
                let total_supply = params.get(0).copied().unwrap_or(1_000_000);
                initial_slots.insert(0, total_supply); // Slot 0: Total Supply
                let creator_id = u64::from_be_bytes(creator.0[12..20].try_into().unwrap_or_default());
                initial_slots.insert(100 + (creator_id % 1000), total_supply); // Slot: Creator balance
            }
            "vault" => {
                initial_slots.insert(0, 0); // Slot 0: Total Staked Assets
                initial_slots.insert(1, 0); // Slot 1: Total Shares Minted
                initial_slots.insert(2, 5); // Slot 2: APY % (Default 5%)
            }
            "uniswap_v2" | "uniswap" => {
                let res_a = params.get(0).copied().unwrap_or(10_000); // Reserve A (e.g. 10,000 ETH)
                let res_b = params.get(1).copied().unwrap_or(20_000_000); // Reserve B (e.g. 20,000,000 USDC)
                let lp_supply = integer_sqrt(res_a.saturating_mul(res_b));
                let creator_id = u64::from_be_bytes(creator.0[12..20].try_into().unwrap_or_default());
                initial_slots.insert(0, res_a); // Slot 0: Reserve Token A
                initial_slots.insert(1, res_b); // Slot 1: Reserve Token B
                initial_slots.insert(2, lp_supply); // Slot 2: Total LP Supply
                initial_slots.insert(100 + (creator_id % 1000), lp_supply); // Creator LP
            }
            "maker_cdp" | "maker" => {
                let eth_price = params.get(0).copied().unwrap_or(2000); // 2000 USD/ETH
                let liq_ratio = params.get(1).copied().unwrap_or(150); // 150% Liquidation Ratio
                initial_slots.insert(0, 0); // Slot 0: Total Collateral (ETH)
                initial_slots.insert(1, 0); // Slot 1: Total Debt (DAI)
                initial_slots.insert(2, eth_price); // Slot 2: Collateral Price
                initial_slots.insert(3, liq_ratio); // Slot 3: Liquidation Ratio %
            }
            "raydium" | "orca" => {
                let res_base = params.get(0).copied().unwrap_or(50_000); // Base: 50,000 SOL
                let res_quote = params.get(1).copied().unwrap_or(7_500_000); // Quote: 7.5M USDC
                initial_slots.insert(0, res_base); // Slot 0: SOL Reserve
                initial_slots.insert(1, res_quote); // Slot 1: USDC Reserve
                initial_slots.insert(2, 0); // Slot 2: Accumulated Fee
            }
            "spl_token" => {
                let total_supply = params.get(0).copied().unwrap_or(100_000_000);
                let decimals = params.get(1).copied().unwrap_or(6);
                let creator_id = u64::from_be_bytes(creator.0[12..20].try_into().unwrap_or_default());
                initial_slots.insert(0, total_supply); // Slot 0: Total Supply
                initial_slots.insert(1, decimals); // Slot 1: Decimals
                initial_slots.insert(2, 0); // Slot 2: Freeze flag (0: unfrozen)
                initial_slots.insert(100 + (creator_id % 1000), total_supply);
            }
            _ => {
                // Custom contract: initialize arbitrary slots from params
                for (i, &p) in params.iter().enumerate() {
                    initial_slots.insert(i as u64, p);
                }
            }
        }

        let info = ContractInfo {
            address,
            name: name.to_string(),
            template: template.to_lowercase(),
            creator,
            created_at_round: round,
        };

        (info, initial_slots)
    }

    pub fn execute_call<F>(
        sender: Address,
        contract: &ContractInfo,
        method: &str,
        args: &[u64],
        read_slot: F,
    ) -> VmExecutionResult
    where
        F: Fn(u64) -> u64,
    {
        let mut slot_writes = HashMap::new();
        let mut balance_deltas = Vec::new();
        let mut logs = Vec::new();
        let gas_used = 21000 + (args.len() as u64 * 500);

        let sender_id = u64::from_be_bytes(sender.0[12..20].try_into().unwrap_or_default());
        let sender_slot = 100 + (sender_id % 1000);

        match contract.template.as_str() {
            "counter" => {
                let current_val = read_slot(0);
                match method.to_lowercase().as_str() {
                    "increment" => {
                        let step = args.get(0).copied().unwrap_or(1);
                        let next = current_val.saturating_add(step);
                        slot_writes.insert(0, next);
                        logs.push(format!("Counter incremented: {} -> {}", current_val, next));
                    }
                    "decrement" => {
                        let step = args.get(0).copied().unwrap_or(1);
                        let next = current_val.saturating_sub(step);
                        slot_writes.insert(0, next);
                        logs.push(format!("Counter decremented: {} -> {}", current_val, next));
                    }
                    "set" => {
                        let val = args.get(0).copied().unwrap_or(0);
                        slot_writes.insert(0, val);
                        logs.push(format!("Counter set to: {}", val));
                    }
                    _ => {
                        logs.push(format!("Unknown counter method: {}", method));
                    }
                }
            }
            "token" => {
                match method.to_lowercase().as_str() {
                    "transfer" => {
                        let recipient_id = args.get(0).copied().unwrap_or(0);
                        let amount = args.get(1).copied().unwrap_or(0);
                        let recipient_slot = 100 + (recipient_id % 1000);

                        let sender_bal = read_slot(sender_slot);
                        let recv_bal = read_slot(recipient_slot);

                        if sender_bal >= amount {
                            slot_writes.insert(sender_slot, sender_bal - amount);
                            slot_writes.insert(recipient_slot, recv_bal + amount);
                            logs.push(format!("Transferred {} tokens from {} to {}", amount, sender, Address::new(recipient_id)));
                        } else {
                            logs.push(format!("Insufficient token balance: {} < {}", sender_bal, amount));
                        }
                    }
                    "mint" => {
                        let recipient_id = args.get(0).copied().unwrap_or(sender_id);
                        let amount = args.get(1).copied().unwrap_or(100);
                        let recipient_slot = 100 + (recipient_id % 1000);

                        let total_supply = read_slot(0);
                        let recv_bal = read_slot(recipient_slot);

                        slot_writes.insert(0, total_supply + amount);
                        slot_writes.insert(recipient_slot, recv_bal + amount);
                        logs.push(format!("Minted {} tokens to {}. New supply: {}", amount, Address::new(recipient_id), total_supply + amount));
                    }
                    _ => {
                        logs.push(format!("Unknown token method: {}", method));
                    }
                }
            }
            "vault" => {
                match method.to_lowercase().as_str() {
                    "deposit" => {
                        let amount = args.get(0).copied().unwrap_or(100);
                        let current_assets = read_slot(0);
                        let current_shares = read_slot(1);

                        // Transfer user balance to vault
                        balance_deltas.push((sender, -(amount as i64)));

                        let shares_to_mint = if current_assets == 0 || current_shares == 0 {
                            amount
                        } else {
                            (amount * current_shares) / current_assets
                        };

                        let user_shares = read_slot(sender_slot);

                        slot_writes.insert(0, current_assets + amount);
                        slot_writes.insert(1, current_shares + shares_to_mint);
                        slot_writes.insert(sender_slot, user_shares + shares_to_mint);

                        logs.push(format!("Vault deposit: {} AETH deposited, {} shares minted for {}", amount, shares_to_mint, sender));
                    }
                    "compound" => {
                        let apy = read_slot(2);
                        let current_assets = read_slot(0);
                        let yield_amount = (current_assets * apy) / 100;
                        slot_writes.insert(0, current_assets + yield_amount);
                        logs.push(format!("Vault compounded: +{} yield generated. Total assets: {}", yield_amount, current_assets + yield_amount));
                    }
                    _ => {
                        logs.push(format!("Unknown vault method: {}", method));
                    }
                }
            }
            "uniswap_v2" | "uniswap" => {
                match method.to_lowercase().as_str() {
                    "swap_a_to_b" | "swap_x_for_y" => {
                        let amount_in = args.get(0).copied().unwrap_or(0);
                        let min_out = args.get(1).copied().unwrap_or(0);
                        let res_a = read_slot(0);
                        let res_b = read_slot(1);
                        if res_a > 0 && res_b > 0 && amount_in > 0 {
                            let amount_in_with_fee = amount_in as u128 * 997;
                            let numerator = amount_in_with_fee * (res_b as u128);
                            let denominator = (res_a as u128 * 1000) + amount_in_with_fee;
                            let amount_out = (numerator / denominator) as u64;

                            if amount_out >= min_out && amount_out < res_b {
                                slot_writes.insert(0, res_a + amount_in);
                                slot_writes.insert(1, res_b - amount_out);
                                logs.push(format!("Uniswap V2 Swap: {} Token A -> {} Token B", amount_in, amount_out));
                            } else {
                                logs.push(format!("Slippage exceeded: expected min {}, got {}", min_out, amount_out));
                            }
                        }
                    }
                    "swap_b_to_a" | "swap_y_for_x" => {
                        let amount_in = args.get(0).copied().unwrap_or(0);
                        let min_out = args.get(1).copied().unwrap_or(0);
                        let res_a = read_slot(0);
                        let res_b = read_slot(1);
                        if res_a > 0 && res_b > 0 && amount_in > 0 {
                            let amount_in_with_fee = amount_in as u128 * 997;
                            let numerator = amount_in_with_fee * (res_a as u128);
                            let denominator = (res_b as u128 * 1000) + amount_in_with_fee;
                            let amount_out = (numerator / denominator) as u64;

                            if amount_out >= min_out && amount_out < res_a {
                                slot_writes.insert(1, res_b + amount_in);
                                slot_writes.insert(0, res_a - amount_out);
                                logs.push(format!("Uniswap V2 Swap: {} Token B -> {} Token A", amount_in, amount_out));
                            }
                        }
                    }
                    "add_liquidity" | "mint_lp" => {
                        let amount_a = args.get(0).copied().unwrap_or(0);
                        let amount_b = args.get(1).copied().unwrap_or(0);
                        let res_a = read_slot(0);
                        let res_b = read_slot(1);
                        let total_lp = read_slot(2);
                        let user_lp = read_slot(sender_slot);

                        let minted_lp = if total_lp == 0 {
                            integer_sqrt(amount_a.saturating_mul(amount_b))
                        } else {
                            std::cmp::min(
                                (amount_a as u128 * total_lp as u128 / res_a as u128) as u64,
                                (amount_b as u128 * total_lp as u128 / res_b as u128) as u64,
                            )
                        };

                        slot_writes.insert(0, res_a + amount_a);
                        slot_writes.insert(1, res_b + amount_b);
                        slot_writes.insert(2, total_lp + minted_lp);
                        slot_writes.insert(sender_slot, user_lp + minted_lp);
                        logs.push(format!("Uniswap V2 Add Liquidity: +{} A, +{} B -> {} LP minted", amount_a, amount_b, minted_lp));
                    }
                    "remove_liquidity" | "burn_lp" => {
                        let lp_amount = args.get(0).copied().unwrap_or(0);
                        let res_a = read_slot(0);
                        let res_b = read_slot(1);
                        let total_lp = read_slot(2);
                        let user_lp = read_slot(sender_slot);

                        if user_lp >= lp_amount && total_lp > 0 {
                            let out_a = (lp_amount as u128 * res_a as u128 / total_lp as u128) as u64;
                            let out_b = (lp_amount as u128 * res_b as u128 / total_lp as u128) as u64;

                            slot_writes.insert(0, res_a.saturating_sub(out_a));
                            slot_writes.insert(1, res_b.saturating_sub(out_b));
                            slot_writes.insert(2, total_lp.saturating_sub(lp_amount));
                            slot_writes.insert(sender_slot, user_lp - lp_amount);
                            logs.push(format!("Uniswap V2 Remove Liquidity: -{} LP -> {} A, {} B", lp_amount, out_a, out_b));
                        }
                    }
                    _ => logs.push(format!("Unknown Uniswap method: {}", method)),
                }
            }
            "maker_cdp" | "maker" => {
                match method.to_lowercase().as_str() {
                    "deposit_collateral" | "deposit" => {
                        let amount = args.get(0).copied().unwrap_or(0);
                        let total_col = read_slot(0);
                        let user_col = read_slot(sender_slot);
                        slot_writes.insert(0, total_col + amount);
                        slot_writes.insert(sender_slot, user_col + amount);
                        logs.push(format!("MakerDAO Deposit: +{} ETH collateral", amount));
                    }
                    "borrow" | "borrow_debt" => {
                        let dai_amount = args.get(0).copied().unwrap_or(0);
                        let price = read_slot(2);
                        let liq_ratio = read_slot(3);
                        let user_col = read_slot(sender_slot);
                        let debt_slot = 200 + (sender_id % 1000);
                        let user_debt = read_slot(debt_slot);

                        let max_debt = if liq_ratio > 0 {
                            (user_col as u128 * price as u128 * 100 / liq_ratio as u128) as u64
                        } else {
                            0
                        };

                        if user_debt + dai_amount <= max_debt {
                            let total_debt = read_slot(1);
                            slot_writes.insert(1, total_debt + dai_amount);
                            slot_writes.insert(debt_slot, user_debt + dai_amount);
                            logs.push(format!("MakerDAO Borrow: {} DAI borrowed against {} ETH", dai_amount, user_col));
                        } else {
                            logs.push(format!("Borrow rejected: Exceeds 150% collateral ratio"));
                        }
                    }
                    "repay" | "repay_debt" => {
                        let dai_amount = args.get(0).copied().unwrap_or(0);
                        let debt_slot = 200 + (sender_id % 1000);
                        let user_debt = read_slot(debt_slot);
                        let repaid = std::cmp::min(user_debt, dai_amount);
                        let total_debt = read_slot(1);
                        slot_writes.insert(1, total_debt.saturating_sub(repaid));
                        slot_writes.insert(debt_slot, user_debt - repaid);
                        logs.push(format!("MakerDAO Repay: {} DAI repaid", repaid));
                    }
                    _ => logs.push(format!("Unknown MakerDAO method: {}", method)),
                }
            }
            "raydium" | "orca" => {
                match method.to_lowercase().as_str() {
                    "swap_base_to_quote" | "swap_coin_for_pc" => {
                        let base_in = args.get(0).copied().unwrap_or(0);
                        let min_quote_out = args.get(1).copied().unwrap_or(0);
                        let res_base = read_slot(0);
                        let res_quote = read_slot(1);

                        if res_base > 0 && res_quote > 0 && base_in > 0 {
                            let base_with_fee = base_in as u128 * 9975;
                            let numerator = base_with_fee * (res_quote as u128);
                            let denominator = (res_base as u128 * 10000) + base_with_fee;
                            let quote_out = (numerator / denominator) as u64;

                            if quote_out >= min_quote_out && quote_out < res_quote {
                                slot_writes.insert(0, res_base + base_in);
                                slot_writes.insert(1, res_quote - quote_out);
                                let fee = (base_in * 25) / 10000;
                                let accum_fee = read_slot(2);
                                slot_writes.insert(2, accum_fee + fee);
                                logs.push(format!("Raydium Swap: {} SOL -> {} USDC (Fee: {} SOL)", base_in, quote_out, fee));
                            }
                        }
                    }
                    "swap_quote_to_base" | "swap_pc_for_coin" => {
                        let quote_in = args.get(0).copied().unwrap_or(0);
                        let min_base_out = args.get(1).copied().unwrap_or(0);
                        let res_base = read_slot(0);
                        let res_quote = read_slot(1);

                        if res_base > 0 && res_quote > 0 && quote_in > 0 {
                            let quote_with_fee = quote_in as u128 * 9975;
                            let numerator = quote_with_fee * (res_base as u128);
                            let denominator = (res_quote as u128 * 10000) + quote_with_fee;
                            let base_out = (numerator / denominator) as u64;

                            if base_out >= min_base_out && base_out < res_base {
                                slot_writes.insert(1, res_quote + quote_in);
                                slot_writes.insert(0, res_base - base_out);
                                logs.push(format!("Raydium Swap: {} USDC -> {} SOL", quote_in, base_out));
                            }
                        }
                    }
                    "add_liquidity" => {
                        let base_in = args.get(0).copied().unwrap_or(0);
                        let quote_in = args.get(1).copied().unwrap_or(0);
                        let res_base = read_slot(0);
                        let res_quote = read_slot(1);
                        slot_writes.insert(0, res_base + base_in);
                        slot_writes.insert(1, res_quote + quote_in);
                        logs.push(format!("Raydium Add Liquidity: +{} Base, +{} Quote", base_in, quote_in));
                    }
                    _ => logs.push(format!("Unknown Raydium method: {}", method)),
                }
            }
            "spl_token" => {
                let global_frozen = read_slot(2) == 1;
                match method.to_lowercase().as_str() {
                    "mint_to" => {
                        let recipient_id = args.get(0).copied().unwrap_or(sender_id);
                        let amount = args.get(1).copied().unwrap_or(0);
                        let total_supply = read_slot(0);
                        let recv_slot = 100 + (recipient_id % 1000);
                        let recv_bal = read_slot(recv_slot);

                        slot_writes.insert(0, total_supply + amount);
                        slot_writes.insert(recv_slot, recv_bal + amount);
                        logs.push(format!("SPL Token MintTo: +{} tokens to account {}", amount, Address::new(recipient_id)));
                    }
                    "transfer_checked" | "transfer" => {
                        let sender_frozen = global_frozen || read_slot(200 + (sender_id % 1000)) == 1;
                        if sender_frozen {
                            logs.push("SPL Token Error: Token account is frozen!".to_string());
                        } else {
                            let recipient_id = args.get(0).copied().unwrap_or(0);
                            let amount = args.get(1).copied().unwrap_or(0);
                            let recv_slot = 100 + (recipient_id % 1000);
                            let sender_bal = read_slot(sender_slot);
                            let recv_bal = read_slot(recv_slot);

                            if sender_bal >= amount {
                                slot_writes.insert(sender_slot, sender_bal - amount);
                                slot_writes.insert(recv_slot, recv_bal + amount);
                                logs.push(format!("SPL Token Transfer: {} tokens from {} to {}", amount, sender, Address::new(recipient_id)));
                            } else {
                                logs.push("SPL Token Error: Insufficient balance".to_string());
                            }
                        }
                    }
                    "freeze" | "freeze_account" => {
                        slot_writes.insert(2, 1);
                        if let Some(&target_acc) = args.get(0) {
                            slot_writes.insert(200 + (target_acc % 1000), 1);
                        }
                        logs.push("SPL Token account frozen".to_string());
                    }
                    "thaw" | "thaw_account" => {
                        slot_writes.insert(2, 0);
                        if let Some(&target_acc) = args.get(0) {
                            slot_writes.insert(200 + (target_acc % 1000), 0);
                        }
                        logs.push("SPL Token account thawed".to_string());
                    }
                    _ => logs.push(format!("Unknown SPL Token method: {}", method)),
                }
            }
            _ => {
                // Custom program execution: write args directly as (slot, value) pairs
                for i in (0..args.len()).step_by(2) {
                    if i + 1 < args.len() {
                        let slot = args[i];
                        let val = args[i + 1];
                        slot_writes.insert(slot, val);
                        logs.push(format!("Custom contract: Slot[{}] = {}", slot, val));
                    }
                }
            }
        }

        VmExecutionResult {
            contract_slot_writes: slot_writes,
            account_balance_deltas: balance_deltas,
            logs,
            gas_used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter_lifecycle() {
        let creator = Address::new(1);
        let (info, initial_slots) = SmartContractEngine::deploy(creator, 0, 1, "MyCounter", "counter", &[10]);
        assert_eq!(initial_slots.get(&0), Some(&10));

        let mut storage = initial_slots;
        let res = SmartContractEngine::execute_call(
            creator,
            &info,
            "increment",
            &[5],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        assert_eq!(res.contract_slot_writes.get(&0), Some(&15));
        storage.extend(res.contract_slot_writes);

        let res2 = SmartContractEngine::execute_call(
            creator,
            &info,
            "decrement",
            &[3],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        assert_eq!(res2.contract_slot_writes.get(&0), Some(&12));
    }

    #[test]
    fn test_token_lifecycle() {
        let creator = Address::new(10);
        let (info, initial_slots) = SmartContractEngine::deploy(creator, 0, 1, "GovToken", "token", &[1_000_000]);
        assert_eq!(initial_slots.get(&0), Some(&1_000_000));

        let mut storage = initial_slots;
        // Mint to user 20
        let res = SmartContractEngine::execute_call(
            creator,
            &info,
            "mint",
            &[20, 500],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        storage.extend(res.contract_slot_writes);
        assert_eq!(storage.get(&0), Some(&1_000_500));
        assert_eq!(storage.get(&(100 + 20)), Some(&500));
    }

    #[test]
    fn test_custom_contract() {
        let creator = Address::new(99);
        let (info, initial_slots) = SmartContractEngine::deploy(creator, 0, 1, "CustomLogic", "custom", &[111, 222, 333]);
        assert_eq!(initial_slots.get(&0), Some(&111));
        assert_eq!(initial_slots.get(&1), Some(&222));
        assert_eq!(initial_slots.get(&2), Some(&333));

        let mut storage = initial_slots;
        let res = SmartContractEngine::execute_call(
            creator,
            &info,
            "set_slots",
            &[5, 9999], // Slot 5 = 9999
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        storage.extend(res.contract_slot_writes);
        assert_eq!(storage.get(&5), Some(&9999));
    }

    #[test]
    fn test_uniswap_v2_amm() {
        let creator = Address::new(1);
        // Deploy Uniswap ETH-USDC pool: 10,000 ETH, 20,000,000 USDC
        let (info, initial_slots) = SmartContractEngine::deploy(
            creator,
            0,
            1,
            "UniswapV2_ETH_USDC",
            "uniswap_v2",
            &[10_000, 20_000_000],
        );

        assert_eq!(initial_slots.get(&0), Some(&10_000));
        assert_eq!(initial_slots.get(&1), Some(&20_000_000));

        let mut storage = initial_slots;
        // User swaps 10 ETH for USDC with min 19,000 USDC
        let trader = Address::new(2);
        let res = SmartContractEngine::execute_call(
            trader,
            &info,
            "swap_a_to_b",
            &[10, 19_000],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        assert_eq!(res.contract_slot_writes.get(&0), Some(&10_010));
        let new_res_b = *res.contract_slot_writes.get(&1).unwrap();
        assert!(new_res_b < 20_000_000);
        let usdc_received = 20_000_000 - new_res_b;
        assert!(usdc_received >= 19_000, "Should receive at least 19,000 USDC");
        storage.extend(res.contract_slot_writes);

        // Add liquidity: +100 ETH, +200,000 USDC
        let lp_res = SmartContractEngine::execute_call(
            creator,
            &info,
            "add_liquidity",
            &[100, 200_000],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        storage.extend(lp_res.contract_slot_writes);
        assert_eq!(storage.get(&0), Some(&10_110));
    }

    #[test]
    fn test_maker_cdp_lending() {
        let creator = Address::new(1);
        // Price 2000 USD/ETH, Liq Ratio 150%
        let (info, initial_slots) = SmartContractEngine::deploy(
            creator,
            0,
            1,
            "MakerDAO_CDP",
            "maker_cdp",
            &[2000, 150],
        );

        let mut storage = initial_slots;
        let borrower = Address::new(5);
        let borrower_slot = 100 + 5;
        let debt_slot = 200 + 5;

        // 1. Deposit 10 ETH collateral ($20,000 value)
        let dep_res = SmartContractEngine::execute_call(
            borrower,
            &info,
            "deposit_collateral",
            &[10],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        storage.extend(dep_res.contract_slot_writes);
        assert_eq!(storage.get(&borrower_slot), Some(&10));

        // 2. Borrow 10,000 DAI (Max allowed is 20,000 / 1.5 = 13,333 DAI)
        let borrow_res = SmartContractEngine::execute_call(
            borrower,
            &info,
            "borrow",
            &[10_000],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        storage.extend(borrow_res.contract_slot_writes);
        assert_eq!(storage.get(&debt_slot), Some(&10_000));

        // 3. Repay 4,000 DAI
        let repay_res = SmartContractEngine::execute_call(
            borrower,
            &info,
            "repay",
            &[4_000],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        storage.extend(repay_res.contract_slot_writes);
        assert_eq!(storage.get(&debt_slot), Some(&6_000));
    }

    #[test]
    fn test_raydium_swap() {
        let creator = Address::new(1);
        // Raydium pool: 50,000 SOL, 7,500,000 USDC
        let (info, initial_slots) = SmartContractEngine::deploy(
            creator,
            0,
            1,
            "Raydium_SOL_USDC",
            "raydium",
            &[50_000, 7_500_000],
        );

        let storage = initial_slots;
        let trader = Address::new(8);
        // Swap 100 SOL for USDC with min 14,000 USDC
        let res = SmartContractEngine::execute_call(
            trader,
            &info,
            "swap_base_to_quote",
            &[100, 14_000],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        assert_eq!(res.contract_slot_writes.get(&0), Some(&50_100));
        let new_quote = *res.contract_slot_writes.get(&1).unwrap();
        assert!(new_quote < 7_500_000);
        assert!(7_500_000 - new_quote >= 14_000);
    }

    #[test]
    fn test_spl_token_standard() {
        let creator = Address::new(1);
        let (info, initial_slots) = SmartContractEngine::deploy(
            creator,
            0,
            1,
            "SPL_USDC",
            "spl_token",
            &[1_000_000, 6],
        );

        let mut storage = initial_slots;
        let user = Address::new(10);
        let user_slot = 100 + 10;
        let creator_slot = 100 + 1;

        // Transfer 250,000 tokens to user
        let tx_res = SmartContractEngine::execute_call(
            creator,
            &info,
            "transfer_checked",
            &[10, 250_000],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        storage.extend(tx_res.contract_slot_writes);
        assert_eq!(storage.get(&creator_slot), Some(&750_000));
        assert_eq!(storage.get(&user_slot), Some(&250_000));

        // Freeze token
        let freeze_res = SmartContractEngine::execute_call(
            creator,
            &info,
            "freeze",
            &[],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        storage.extend(freeze_res.contract_slot_writes);
        assert_eq!(storage.get(&2), Some(&1));

        // Attempt transfer while frozen -> should fail
        let fail_res = SmartContractEngine::execute_call(
            user,
            &info,
            "transfer_checked",
            &[20, 100],
            |slot| *storage.get(&slot).unwrap_or(&0),
        );
        assert!(fail_res.contract_slot_writes.is_empty(), "Transfer while frozen should produce no writes");
    }
}


