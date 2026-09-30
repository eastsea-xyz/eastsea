// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {AtomicSwap} from "./AtomicSwap.sol";

/// Deploy this same admin-less implementation on Ethereum, BNB, or Base.
/// It uses only EVM primitives and needs no chain-specific constructor data.
contract AtomicSwapEVM is AtomicSwap {}
