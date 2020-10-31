// near_sdk provides lots of support for easily writing performant contracts including a custom
// binary serialization / deserialization format called [borsh] for storing contract state
// efficiently on chain to reduce both [storage] and [gas] costs.
//
//   [borsh]: https://borsh.io/
//   [storage]: https://docs.near.org/docs/concepts/storage#other-ways-to-keep-costs-down
//   [gas]: https://docs.near.org/docs/concepts/gas
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};

//  Clients of the contracts may require JSON formats for arguments & return types. Since JSON is
//  limited to JavaScript number support (10^53), we need to wrap Rust u64 and u128 with some helper
//  functionality that returns these numbers as JSON strings. The difference is the uppercase "U"
//  on wrapped types vs the lowercase "u" on Rust native types.
use near_sdk::json_types::{U128, U64};

// near_sdk exposes the environment through `env` which developers can use to interrogate the NEAR
// Runtime for information like the signer of the current transaction, the state of the contract,
// and more. near_bindgen will be explained where used lower in this document. AccountId, Balance
// and EpochHeight are all types that are native to NEAR protocol and represent what you expect
// them to. Check out the nearcore source code for more details about these types like AccountId
// here: https://github.com/near/nearcore/blob/master/core/primitives/src/types.rs#L15
use near_sdk::{env, near_bindgen, AccountId, Balance, EpochHeight};

// When choosing to organize how data is stored by your contract, it's important to first decide
// whether you want to use "ACCOUNT storage" or "contract STATE storage". "ACCOUNT storage" is a
// key-value structure which is ideal for managing large contract state or cases with sparse access
// to contract data (ie. you only need a few pieces of data on occasion). This state is only
// deserialized when accessed and otherwise remains untouched. You would either write to and read from
// directly using the Storage interface or choose a more appropriate abstraction from among the
// list of available [collections], like a [PersistentVector] or [UnorderedMap]. From the source
// code:  UnorderedMap is a map implemented on a trie. Unlike `std::collections::HashMap` the keys
// in this map are not hashed but are instead serialized. "contract STATE storage" is a serialized
// representation of the contract struct (the one decorated with #[near_bindgen] below). Using
// contract state storage is more efficient when contract data is 1. small and known to never risk
// growing unprocessably large, and 2. when operations on the data are infrequent. Since this state
// is deserialized in its entirety every time a contract method is called, it's important to keep
// it small.
//
//   [collections]: https://github.com/near/near-sdk-rs/tree/master/near-sdk/src/collections
//   [PersistentVector]: https://github.com/near/near-sdk-rs/blob/master/near-sdk/src/collections/vector.rs
//   [UnorderedMap]: https://github.com/near/near-sdk-rs/blob/master/near-sdk/src/collections/unordered_map.rs
use std::collections::HashMap;

// Rust allows us to define our own custom memory allocation mechanism to make memory management as
// efficient as possible and wee_alloc, the "Wasm-Enabled, Elfin (small) Allocator" has good
// performance when compiled to Wasm (https://github.com/rustwasm/wee_alloc)
#[global_allocator]
static ALLOC: near_sdk::wee_alloc::WeeAlloc = near_sdk::wee_alloc::WeeAlloc::INIT;

type WrappedTimestamp = U64;

// Wrap a struct in `#[near_bindgen]` and it generates a smart contract compatible with the NEAR
// blockchain (https://github.com/near/near-sdk-rs/blob/master/near-sdk-macros/src/lib.rs#L13)
/// Voting contract for unlocking transfers. Once the majority of the stake holders agree to
/// unlock transfer, the time will be recorded and the voting ends.
#[near_bindgen]
// Contract state should be serialized to store it on chain. This contract is serialized using
// Borsh, described more above.
#[derive(BorshDeserialize, BorshSerialize)]
pub struct VotingContract {
    // examples of storing primitive values in contract state
    some_number: u32,
    some_bool: bool,

    // an example of storing a map of custom types (AccountId -> Balance)
    /// How much each validator votes
    votes: HashMap<AccountId, Balance>,

    // an example of storing a custom type (Balance)
    /// Total voted balance so far.
    total_voted_stake: Balance,
    /// When the voting ended. `None` means the poll is still open.
    result: Option<WrappedTimestamp>,
    /// Epoch height when the contract is touched last time.
    last_epoch_height: EpochHeight,
}

// This bit of code is part of a larger context where we have decided to control contract
// initialization by decorating a specific method with the #[init] macro (see below). By default
// all contract methods will try to initialize contract state if not already initialized (since
// it's possible to call any public methods on a deployed contract) but in this case we want to
// avoid that behavior.
impl Default for VotingContract {
    fn default() -> Self {
        env::panic(b"Voting contract should be initialized before usage")
    }
}

// Decorating an implementation (`impl`) with #[near_bindgen] wraps all public methods with some
// extra code that handles serialization / deserialization for method arguments and return types,
// panics if money is attached unless the method is decorated with #[payable], and more.
#[near_bindgen]
impl VotingContract {
    // Decorating a public method with #[init] will add some machinery to "initialize the contract"
    // by serializing & saving the returned value (in this case a struct) into the contract's on-chain
    // state. You can add an #[init] decorator to any public method to mark it as something like a
    // "constructor" for the contract -- there's nothing special about the `new()` method name here,
    // it could just as well be `old()` and would work fine. `new()` is a convention in NEAR-built
    // contracts because it sets the correct expectation for anyone reading the code.
    // This method uses a reference to the environment to assert that the state does not already
    // exist. We don't want to accidentally re-initialize the contract at some later time and blow
    // away any valuable state that has been captured.
    #[init]
    pub fn new() -> Self {
        assert!(!env::state_exists(), "The contract is already initialized");
        VotingContract {
            votes: HashMap::new(),
            total_voted_stake: 0,
            result: None,
            last_epoch_height: 0,
        }
    }

    // Public methods (`pub fn`) will be exposed as contract methods avaliable for clients to call.
    // If a method borrows self -- that is, the first argument in the list is `&self`, a Rust
    // thing that basically means that it "safely takes a temporary reference to self" -- then you
    // can be sure that it will interact with contract state but NOT change contract state.
    // If the first argument is a mutable reference to self (`&mut self`) then you know that the
    // function will both interact with contract state and that it WILL change it.
    //
    // This method, `ping`, will update the votes according to current stake of validators.
    pub fn ping(&mut self) {
        assert!(self.result.is_none(), "Voting has already ended");
        let cur_epoch_height = env::epoch_height();
        if cur_epoch_height != self.last_epoch_height {
            let votes = std::mem::take(&mut self.votes);
            self.total_voted_stake = 0;
            for (account_id, _) in votes {
                let account_current_stake = env::validator_stake(&account_id);
                self.total_voted_stake += account_current_stake;
                if account_current_stake > 0 {
                    self.votes.insert(account_id, account_current_stake);
                }
            }
            self.check_result();
            self.last_epoch_height = cur_epoch_height;
        }
    }

    /// Check whether the voting has ended.
    fn check_result(&mut self) {
        assert!(
            self.result.is_none(),
            "check result is called after result is already set"
        );
        let total_stake = env::validator_total_stake();
        if self.total_voted_stake > 2 * total_stake / 3 {
            self.result = Some(U64::from(env::block_timestamp()));
        }
    }

    // Methods can take arguments which will be deserialized from JSON and return types which will
    // be serialized to JSON if called from an external context. Making internal method calls
    // within a Rust context doesn't require the same serialization / deserialization overhead.
    //
    // Internally we can call contracts using positional arguments. Via any external interface
    // (simulation tests, near-api-js, etc), we have to pass JSON objects which get deserialized
    // into the fn args as needed.
    /// Method for validators to vote or withdraw the vote.
    /// Votes for if `is_vote` is true, or withdraws the vote if `is_vote` is false.
    pub fn vote(&mut self, is_vote: bool) {
        self.ping();
        if self.result.is_some() {
            return;
        }
        let account_id = env::predecessor_account_id();
        let account_stake = if is_vote {
            let stake = env::validator_stake(&account_id);
            assert!(stake > 0, "{} is not a validator", account_id);
            stake
        } else {
            0
        };
        let voted_stake = self.votes.remove(&account_id).unwrap_or_default();
        assert!(
            voted_stake <= self.total_voted_stake,
            "invariant: voted stake {} is more than total voted stake {}",
            voted_stake,
            self.total_voted_stake
        );
        self.total_voted_stake = self.total_voted_stake + account_stake - voted_stake;
        if account_stake > 0 {
            self.votes.insert(account_id, account_stake);
            self.check_result();
        }
    }

    /// Get the timestamp of when the voting finishes. `None` means the voting hasn't ended yet.
    pub fn get_result(&self) -> Option<WrappedTimestamp> {
        self.result.clone()
    }

    /// Returns current a pair of `total_voted_stake` and the total stake.
    /// Note: as a view method, it doesn't recompute the active stake. May need to call `ping` to
    /// update the active stake.
    pub fn get_total_voted_stake(&self) -> (U128, U128) {
        (
            self.total_voted_stake.into(),
            env::validator_total_stake().into(),
        )
    }

    /// Returns all active votes.
    /// Note: as a view method, it doesn't recompute the active stake. May need to call `ping` to
    /// update the active stake.
    pub fn get_votes(&self) -> HashMap<AccountId, U128> {
        self.votes
            .iter()
            .map(|(account_id, stake)| (account_id.clone(), (*stake).into()))
            .collect()
    }

    // Decorating a contract method with the #[payable] macro will allow the method to accept
    // attached native NEAR tokens. If money is sent to a method without adding the #[payable]
    // macro then the method will panic.
    #[payable]
    pub fn show_me_the_money(&mut self) {
        unimplemented!();
    }

    // Methods which are not marked as public will not be exposed as part of the contract interface
    fn private_method() {
        unimplemented!();
    }

    // Sometimes a method should be available to the contract via promise call which requires that
    // it is exposed on the contract but not callable by anyone other than the contract itself.
    // This is a common pattern in NEAR which can be hand-crafted with a few lines of code but the
    // `#[private]` macro makes it more readable (see near-sdk-rs readme:
    // https://github.com/near/near-sdk-rs/blob/master/README.md)
    // THIS IS A PENDING FEATURE, not available at time of writing in near-sdk-rs 2.0.0
    #[private]
    pub fn contract_private_method() {
        unimplemented!();
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::MockedBlockchain;
    use near_sdk::{testing_env, VMContext};
    use std::collections::HashMap;
    use std::iter::FromIterator;

    fn get_context(predecessor_account_id: AccountId) -> VMContext {
        get_context_with_epoch_height(predecessor_account_id, 0)
    }

    fn get_context_with_epoch_height(
        predecessor_account_id: AccountId,
        epoch_height: EpochHeight,
    ) -> VMContext {
        VMContext {
            current_account_id: "alice_near".to_string(),
            signer_account_id: "bob_near".to_string(),
            signer_account_pk: vec![0, 1, 2],
            predecessor_account_id,
            input: vec![],
            block_index: 0,
            block_timestamp: 0,
            account_balance: 0,
            account_locked_balance: 0,
            storage_usage: 1000,
            attached_deposit: 0,
            prepaid_gas: 2 * 10u64.pow(14),
            random_seed: vec![0, 1, 2],
            is_view: false,
            output_data_receivers: vec![],
            epoch_height,
        }
    }

    #[test]
    #[should_panic(expected = "is not a validator")]
    fn test_nonvalidator_cannot_vote() {
        let context = get_context("bob.near".to_string());
        let validators = HashMap::from_iter(
            vec![
                ("alice_near".to_string(), 100),
                ("bob_near".to_string(), 100),
            ]
            .into_iter(),
        );
        testing_env!(context, Default::default(), Default::default(), validators);
        let mut contract = VotingContract::new();
        contract.vote(true);
    }

    #[test]
    #[should_panic(expected = "Voting has already ended")]
    fn test_vote_again_after_voting_ends() {
        let context = get_context("alice.near".to_string());
        let validators = HashMap::from_iter(vec![("alice.near".to_string(), 100)].into_iter());
        testing_env!(context, Default::default(), Default::default(), validators);
        let mut contract = VotingContract::new();
        contract.vote(true);
        assert!(contract.result.is_some());
        contract.vote(true);
    }

    #[test]
    fn test_voting_simple() {
        let context = get_context("test0".to_string());
        let validators = (0..10)
            .map(|i| (format!("test{}", i), 10))
            .collect::<HashMap<_, _>>();
        testing_env!(
            context,
            Default::default(),
            Default::default(),
            validators.clone()
        );
        let mut contract = VotingContract::new();

        for i in 0..7 {
            let mut context = get_context(format!("test{}", i));
            testing_env!(
                context.clone(),
                Default::default(),
                Default::default(),
                validators.clone()
            );
            contract.vote(true);
            context.is_view = true;
            testing_env!(
                context,
                Default::default(),
                Default::default(),
                validators.clone()
            );
            assert_eq!(
                contract.get_total_voted_stake(),
                (U128::from(10 * (i + 1)), U128::from(100))
            );
            assert_eq!(
                contract.get_votes(),
                (0..=i)
                    .map(|i| (format!("test{}", i), U128::from(10)))
                    .collect::<HashMap<_, _>>()
            );
            assert_eq!(contract.votes.len() as u128, i + 1);
            if i < 6 {
                assert!(contract.result.is_none());
            } else {
                assert!(contract.result.is_some());
            }
        }
    }

    #[test]
    fn test_voting_with_epoch_change() {
        let validators = (0..10)
            .map(|i| (format!("test{}", i), 10))
            .collect::<HashMap<_, _>>();
        let context = get_context("test0".to_string());
        testing_env!(
            context,
            Default::default(),
            Default::default(),
            validators.clone()
        );
        let mut contract = VotingContract::new();

        for i in 0..7 {
            let context = get_context_with_epoch_height(format!("test{}", i), i);
            testing_env!(
                context,
                Default::default(),
                Default::default(),
                validators.clone()
            );
            contract.vote(true);
            assert_eq!(contract.votes.len() as u64, i + 1);
            if i < 6 {
                assert!(contract.result.is_none());
            } else {
                assert!(contract.result.is_some());
            }
        }
    }

    #[test]
    fn test_validator_stake_change() {
        let mut validators = HashMap::from_iter(vec![
            ("test1".to_string(), 40),
            ("test2".to_string(), 10),
            ("test3".to_string(), 10),
        ]);
        let context = get_context_with_epoch_height("test1".to_string(), 1);
        testing_env!(
            context,
            Default::default(),
            Default::default(),
            validators.clone()
        );

        let mut contract = VotingContract::new();
        contract.vote(true);
        validators.insert("test1".to_string(), 50);
        let context = get_context_with_epoch_height("test2".to_string(), 2);
        testing_env!(
            context,
            Default::default(),
            Default::default(),
            validators.clone()
        );
        contract.ping();
        assert!(contract.result.is_some());
    }

    #[test]
    fn test_withdraw_votes() {
        let validators =
            HashMap::from_iter(vec![("test1".to_string(), 10), ("test2".to_string(), 10)]);
        let context = get_context_with_epoch_height("test1".to_string(), 1);
        testing_env!(
            context,
            Default::default(),
            Default::default(),
            validators.clone()
        );
        let mut contract = VotingContract::new();
        contract.vote(true);
        assert_eq!(contract.votes.len(), 1);
        let context = get_context_with_epoch_height("test1".to_string(), 2);
        testing_env!(
            context,
            Default::default(),
            Default::default(),
            validators.clone()
        );
        contract.vote(false);
        assert!(contract.votes.is_empty());
    }

    #[test]
    fn test_validator_kick_out() {
        let mut validators = HashMap::from_iter(vec![
            ("test1".to_string(), 40),
            ("test2".to_string(), 10),
            ("test3".to_string(), 10),
        ]);
        let context = get_context_with_epoch_height("test1".to_string(), 1);
        testing_env!(
            context,
            Default::default(),
            Default::default(),
            validators.clone()
        );

        let mut contract = VotingContract::new();
        contract.vote(true);
        assert_eq!((contract.get_total_voted_stake().0).0, 40);
        validators.remove(&"test1".to_string());
        let context = get_context_with_epoch_height("test2".to_string(), 2);
        testing_env!(
            context,
            Default::default(),
            Default::default(),
            validators.clone()
        );
        contract.ping();
        assert_eq!((contract.get_total_voted_stake().0).0, 0);
    }
}
