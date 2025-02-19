// Ensure we're `no_std` when compiling for Wasm.
#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::{decl_error, decl_event, decl_module, decl_storage, ensure};
use frame_system::{ensure_root, ensure_signed};
use primitives::{AssetId, Balance, EraIndex, SocketIndex};
use sp_runtime::{DispatchError, DispatchResult, Percent};
use sp_std::prelude::*;
mod math;
pub mod weights;
pub use weights::WeightInfo;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// The module configuration trait.
pub trait Config: frame_system::Config {
	/// The overarching event type.
	type Event: From<Event<Self>> + Into<<Self as frame_system::Config>::Event>;

	type WeightInfo: WeightInfo;
}

decl_module! {
	pub struct Module<T: Config> for enum Call where origin: T::Origin {
		type Error = Error<T>;

		fn deposit_event() = default;


		// REVIEW: Use `///` instead of `//` to make these doc comments that are part of the crate documentation.
		// Register a new Provider.
		// Fails with `ProviderAlreadyRegistered` if this Provider (identified by `origin`) has already been registered.
		#[weight = 10_000]
		pub fn register_operator(origin, _socket: SocketIndex, _who: T::AccountId) -> DispatchResult {
			ensure_root(origin)?;
			Providers::<T>::insert(&_who, true);
			Sockets::<T>::insert(_socket, _who.clone());
			Oracles::<T>::insert(_who.clone(), _socket);
			Self::deposit_event(RawEvent::ProviderRegistered(_who));

			Ok(())
		}

		// Unregisters an existing Provider
		// TODO check weight
		#[weight = 10_000]
		pub fn deregister_operator(origin, _socket: SocketIndex, _who: T::AccountId) -> DispatchResult {
			ensure_root(origin)?;
			Providers::<T>::remove(&_who);
			Sockets::<T>::remove(_socket);
			Oracles::<T>::remove(_who.clone());
			Self::deposit_event(RawEvent::ProviderDeregistered(_who));

			Ok(())
		}

		#[weight = 0]
		fn report(origin, _socket: SocketIndex, _id: AssetId, _price: Balance) -> DispatchResult {
			let who : <T as frame_system::Config>::AccountId = ensure_signed(origin)?;
			ensure!(Providers::<T>::contains_key(who.clone()), Error::<T>::WrongProvider);
			ensure!(Sockets::<T>::get(_socket) == who.clone(), Error::<T>::WrongSocket);
			let results = match Self::asset_price(_id) {
				Some(mut x) => {
					if x.len() != Self::provider_count() as usize {
						let oracles = Self::provider_count();
						let mut batch = vec!{0; oracles as usize};
						batch[_socket as usize] = _price;
						batch
					} else {
						  x[_socket as usize] = _price;
						  x
					}
				},
				_ => {
				  let oracles = Self::provider_count();
				  let mut batch = vec!{0; oracles as usize};
				  batch[_socket as usize] = _price;
				  batch
				}
			  };
			Prices::insert(_id, results);
			Self::deposit_event(RawEvent::PriceSubmitted(_socket, who, _price));

			Ok(())
		}

		/// Slash the validator for a given amount of balance. This can grow the value
		/// For now, it just checks the value is an outlier and excludes from the provider slot
		/// Effects will be felt at the beginning of the next era.
		///
		///
		/// # <weight>
		/// ----------
		/// Weight: O(1)
		/// DB Weight:
		/// - Read: Sockets, Prices
		/// - Write:  Sockets New Account, Sockets Old Account
		/// # </weight>
		#[weight = 10_000]
		fn slash(origin, _socket: SocketIndex, _id: AssetId) -> DispatchResult {
			let batch = Prices::get(_id).unwrap();
			let value = batch[_socket as usize];
			let det = Self::determine_outlier(batch, value);
			ensure!(det, Error::<T>::NotOutlier);
			// Add provider to the slash list of the current era
			let provider = Self::provider_at(_socket);
			Slashes::<T>::insert(1, vec!{provider});
			// remove provider from the slot
			Sockets::<T>::remove(_socket);
			Ok(())
		}

		#[weight = 10_000]
		fn remove_batch(origin, _id: AssetId) {
			ensure_root(origin)?;

			Prices::remove(_id);
		}

		/// Sets the ideal number of validators.
		///
		/// The dispatch origin must be Root.
		///
		/// # <weight>
		/// Weight: O(1)
		/// Write: Validator Count
		/// # </weight>
		#[weight = T::WeightInfo::set_validator_count()]
		fn set_validator_count(origin, #[compact] new: u32) {
			ensure_root(origin)?;
			ProviderCount::put(new);
		}

		/// Increments the ideal number of validators.
		///
		/// The dispatch origin must be Root.
		///
		/// # <weight>
		/// Same as [`set_validator_count`].
		/// # </weight>
		#[weight = T::WeightInfo::set_validator_count()]
		fn increase_validator_count(origin, #[compact] additional: u32) {
			ensure_root(origin)?;
			ProviderCount::mutate(|n| *n += additional);
		}

		/// Scale up the ideal number of validators by a factor.
		///
		/// The dispatch origin must be Root.
		///
		/// # <weight>
		/// Same as [`set_validator_count`].
		/// # </weight>
		#[weight = T::WeightInfo::set_validator_count()]
		fn scale_validator_count(origin, factor: Percent) {
			ensure_root(origin)?;
			ProviderCount::mutate(|n| *n += factor * *n);
		}


	}
}

decl_event! {
	pub enum Event<T> where
		<T as frame_system::Config>::AccountId,
	{
		// A new operator has been registered
		ProviderRegistered(AccountId),

		// An existing operator has been unregistered
		ProviderDeregistered(AccountId),

		// Price reported by an oracle provider
		PriceSubmitted(SocketIndex, AccountId, u128),
	}
}

decl_error! {
	pub enum Error for Module<T: Config>  {
		/// Error names should be descriptive.
		NoneValue,
		/// Manipulating an unknown operator
		UnknownProvider,
		/// Manipulating an unknown request
		UnknownRequest,
		/// Not the expected operator
		WrongProvider,
		/// An operator is already registered.
		ProviderAlreadyRegistered,
		/// Callback cannot be deserialized
		UnknownCallback,
		/// Fee provided does not match minimum required fee
		InsufficientFee,
		/// Price does not exist
		PriceDoesNotExist,
		/// Wrong socket to submit
		WrongSocket,
		/// Outlier not determined
		NotOutlier
	}
}

decl_storage! {
	trait Store for Module<T: Config> as Oracle {

		// A set of all registered Provider
		pub Providers get(fn operator): map hasher(blake2_128_concat) <T as frame_system::Config>::AccountId => bool;

		// Price batch from oracle providers
		pub Prices get(fn asset_price): map hasher(blake2_128_concat) AssetId =>  Option<Vec<Balance>>;

		// Oracles: key as account id, value as oracle socket index
		pub Oracles get(fn oracle): map hasher(blake2_128_concat)  <T as frame_system::Config>::AccountId  => Option<SocketIndex>;

		// Sockets: key as the oracle socket index, value as the oracle provider
		pub Sockets get(fn provider_at): map hasher(blake2_128_concat) SocketIndex => <T as frame_system::Config>::AccountId;

		// Slash: key as the oracle socket index, value as the array of slashed accounts
		pub Slashes get(fn slashes_at): map hasher(blake2_128_concat) EraIndex => Vec<<T as frame_system::Config>::AccountId>;

		/// The ideal number of staking participants.
		pub ProviderCount get(fn provider_count) config(): u32;

	} add_extra_genesis {
		config(oracles):
			Vec<<T as frame_system::Config>::AccountId>;
		build(|config: &GenesisConfig<T>| {
			for oracle in &config.oracles {
				Providers::<T>::insert(oracle, true);
			}
		});
	}
}

// The main implementation block for the module.
impl<T: Config> Module<T> {
	pub fn price(id: AssetId) -> sp_std::result::Result<Balance, DispatchError> {
		match Self::asset_price(id) {
			Some(reports) => {
				// get median value
				let median = Self::get_median(reports);
				return Ok(median)
			},
			None => return Err(DispatchError::from(crate::Error::<T>::PriceDoesNotExist).into()),
		}
	}

	pub fn determine_outlier(batch: Vec<Balance>, value: Balance) -> bool {
		let processed = Self::preprocess(batch);
		let len = processed.len();
		let mid = len / 2;
		let quartile = mid / 2;
		let q3 = mid + quartile;
		let q1 = mid - quartile;
		let iqr = 3 * (processed[q3] - processed[q1]) / 2;
		return processed[q3] + iqr < value || processed[q1] - iqr > value
	}

	pub fn get_median(batch: Vec<Balance>) -> Balance {
		let processed = Self::preprocess(batch);
		let mid = processed.len() / 2;
		processed[mid]
	}

	pub fn preprocess(mut batch: Vec<Balance>) -> Vec<u128> {
		batch.retain(|&i| i != 0);
		batch.sort();
		batch
	}
}
