// near_sdk provides lots of support for easily writing performant contracts including

// a custom binary serialization / deserialization format for storing contract state efficiently on chain to reduce storage costs and also make serialization operations computationally efficient (to reduce compute costs, ie. less gas is used than a JSON serialization format, for example)
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};

//  in some cases, due to limitations of JSON and JavaScript for large numbers, ie. `U64` is a wrapped type that includes an internal `u64` representation but when returned from a method will be converted to a string for compatibility with client-side applications. clients of the contracts will require JSON formats for return types like numbers.  since JSON is limited to JavaScript number support (10^53) we need to wrap Rust u64 and u128 with some helper functionality that returns these numbers as JSON strings.  the difference is the uppercase "U" on wrapped types vs the lowercase "u" on Rust native types (which these types use to maintain a Rust-compatible internal representation)
use near_sdk::json_types::{U128, U64};

// near_sdk exposes the environment through env which developers can use to interrogate the NEAR Runtime for information like the signer of the current transaction, the state of the contract, etc.  near_bindgen is explained in context further along in this document.  AccountId, Balance and EpochHeight are all types that are native to NEAR protocol and represent what you expect them to.  check out the nearcore source code for more details about these types like AccountId here: https://github.com/near/nearcore/blob/master/core/primitives/src/types.rs#L15
use near_sdk::{env, near_bindgen, AccountId, Balance, EpochHeight};

// when choosing to organize how data is stored by your contract, it's important to first decide whether you want to use "ACCOUNT storage" or "contract STATE storage".
// "ACCOUNT storage" is a key-value structure which is ideal for managing large contract state or cases with sparse access to contract data (ie. you only need a few pieces of data on occasion).  this state is only deserialized on access and otherwise remains untouched.  you would either write to and read from directly using the Storage interface or choose a more appropriate abstractions from among the list of available collections (https://github.com/near/near-sdk-rs/tree/master/near-sdk/src/collections) like a PersistentVector or UnorderedMap. from the source code:  UnorderedMap is a map implemented on a trie. Unlike `std::collections::HashMap` the keys in this map are not hashed but are instead serialized. (https://github.com/near/near-sdk-rs/blob/master/near-sdk/src/collections/unordered_map.rs)
// "contract STATE storage" is a serialized representation of the contract struct (the one decorated with #[near_bindgen] below).  using contract state storage is more efficient when contract data is small (or at least bounded, ie. limited) and operations on the data are infrequent (or at least bounded).  since this state is deserialized in its entirety every time a contract method is called, it's important to keep it small.
use std::collections::HashMap;

// Rust allows us to define our own custom memory allocation mechanism to make memory management as efficient as possible and wee_alloc, the "Wasm-Enabled, Elfin (small) Allocator" has good performance when compiled to Wasm (https://github.com/rustwasm/wee_alloc)
#[global_allocator]
static ALLOC: near_sdk::wee_alloc::WeeAlloc = near_sdk::wee_alloc::WeeAlloc::INIT;

// near_sdk provides wrapped types that allow us to return JSON-compatible formats for big numbers. use these types when contract methods return large numbers.
type WrappedTimestamp = U64;

// Wrap a struct in `#[near_bindgen]` and it generates a smart contract compatible with the NEAR blockchain.
// if this macro is used on the struct then it exposes the execution environment.  if the macro is used on an impl section then it wraps public methods of the implementation with contract-compatible code like JSON / Borsch serialization depending on method return types (https://github.com/near/near-sdk-rs/blob/master/near-sdk-macros/src/lib.rs#L13)
#[near_bindgen]
// contract state should be serialized to store it on chain.  borsh is an efficient binary representation developed by the NEAR collective for this purpose.
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

// override the default behavior of all methods in the implementation (which is to initialize state if it doesn't exist).  this is added because the developer chose to use the #[init] decorator to allow fine grained control over contract state initialization. this bit of code is part of a larger context where we have decided to control contract initializing by decorating some other method (see below) with the #[init] macro.  by default all contract methods will try to initialize contract state if not already initialized (since it's possible to call any public methods on a deployed contract) but in this case we want to avoid that behavior
impl Default for VotingContract {
    fn default() -> Self {
        env::panic(b"Voting contract should be initialized before usage")
    }
}

// decorating an implementation with #[near_bindgen] wraps all public methods with some extra code that handles serialization / deserialization or method parameters and return types, panics if money is attached unless the method is decorated with #[payable], etc.
#[near_bindgen]
impl VotingContract {
    // decorating a public method with #[init] will add some machinery to "initialize the contract" by serializing the struct into contract state (saving the return value of this method on chain). you can add the #[init] decorator to any of the public methods to mark it as something like a "constructor" for the contract.  note there's nothing special about the `new()` method name here, it could just as well be `old()` and would work fine ... but `new()` sets the correct expectation for anyone reading the code.
    // this method uses a reference to the environment to assert that the state does not already exist.  we don't want to accidentally re-initialize the contract at some later time and blow away any valuable state that has been captured.
    //  expects that the method it decorates will return an instance of the `struct` which is being referenced by this implementation section. the return value will be serialized on chain.
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

    // public methods will be exposed as contract methods avaliable for clients to call
    // if a method borrows (that's a Rust thing that basically means "safely takes a temporary reference to") self then you can be sure that it will interact with contract state but NOT change contract state
    // same as above but borrowing a mutable reference which means it will interact with contract state AND that it WILL change contract state
    // note that #[near_bindgen] adds code to each method of the contract which reacts to the presence of self or &mut self.  the additional code first tries to deserialize contract state.  contract state is stored in a special key STATE.  near_sdk will call Default() if state does not exist (calls env::state_exists()), otherwise will call Default() as part of preparing to run a method
    /// Ping to update the votes according to current stake of validators.
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

    // methods can take arguments which will be deserialized from JSON and return types which will be serialized to JSON if called from an external context.  making internal method calls within a Rust context doesn't require the same serialization / deserialization overhead of course
    // internally we can call contracts using positional arguments. externally, via some external interface (simulation tests, via near-api-js, etc). raw Wasm doesn't know about types in the interface so no positional args allowed, we have to pass JSON objects which get deserialized into the fn args as needed. we parse inputs for these methods completely differnetly. you can run macro expansion on the compiler to see raw code generated per method and see what's happening behind the scenes before we get to body of method
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

    // decorating a contract method with the #[payable] macro will allow the method to accept attached native NEAR tokens without a panic.  if money is sent to a method without adding the #[payable] macro then the method will panic.
    #[payable]
    pub fn show_me_the_money(&mut self) {
        unimplemented!();
    }

    // methods which are not marked as public will not be exposed as part of the contract interface
    fn private_method() {
        unimplemented!();
    }
    // sometimes a method should be available to the contract via promise call which requires that it is exposed on the contract but not callable by anyone other than the contract itself.  this is a common pattern in NEAR which can be hand-crafted with a few lines of code but this macro makes it more readable (see near-sdk-rs readme: https://github.com/near/near-sdk-rs/blob/master/README.md)
    // THIS IS A PENDING FEATURE, not available in near-sdk-rs 2.0.0 at time of writing
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
