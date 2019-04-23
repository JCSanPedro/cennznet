// TODO: add legal and license info.

//! Generic asset module for runtime.
//!
//! This module provides asset/token features, which includes issuing a token,
//! interacting with tokens such as transferring, spending by other parties.
//!
//! Generic asset module is the foundation of other token related modules/features.
//! It supports dual token economy in CENNZnet, dApp tokens and their token economy.

#![cfg_attr(not(feature = "std"), no_std)]

use parity_codec::{Decode, Encode, HasCompact};

use primitives::traits::{
	As, CheckedAdd, CheckedSub, MaybeSerializeDebug, Member, One, Saturating, SimpleArithmetic, Zero,
};

use rstd::prelude::*;
use rstd::{cmp, result};
use support::dispatch::Result;
use support::{
	additional_traits::{ChargeFee, DummyChargeFee},
	decl_event, decl_module, decl_storage, ensure,
	traits::{
		Currency, ExistenceRequirement, Imbalance, LockIdentifier, LockableCurrency, ReservableCurrency,
		SignedImbalance, UpdateBalanceOutcome, WithdrawReason, WithdrawReasons,
	},
	Parameter, StorageDoubleMap, StorageMap, StorageValue,
};
use system::ensure_signed;

mod tests;

pub use self::imbalances::{NegativeImbalance, PositiveImbalance};

pub trait Trait: system::Trait {
	type Balance: Parameter
		+ Member
		+ SimpleArithmetic
		+ Default
		+ Copy
		+ As<usize>
		+ As<u64>
		+ As<u128>
		+ MaybeSerializeDebug;
	type AssetId: Parameter + Member + SimpleArithmetic + Default + Copy + As<u32>;
	type ChargeFee: ChargeFee<Self::AccountId, Amount = Self::Balance>;
	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
}

pub trait Subtrait: system::Trait {
	type Balance: Parameter
		+ Member
		+ SimpleArithmetic
		+ Default
		+ Copy
		+ As<usize>
		+ As<u64>
		+ As<u128>
		+ MaybeSerializeDebug;
	type AssetId: Parameter + Member + SimpleArithmetic + Default + Copy + As<u32>;
}

impl<T: Trait> Subtrait for T {
	type Balance = T::Balance;
	type AssetId = T::AssetId;
}

/// Asset creation options.
#[cfg_attr(feature = "std", derive(Debug))]
#[derive(Clone, Encode, Decode, PartialEq, Eq)]
pub struct AssetOptions<Balance: HasCompact, AccountId> {
	#[codec(compact)]
	pub initial_issuance: Balance,
	pub permissions: PermissionLatest<AccountId>,
}

#[cfg_attr(feature = "std", derive(Debug))]
#[derive(Clone, Encode, Decode, PartialEq, Eq)]
pub enum Owner<AccountId> {
	None,
	Address(AccountId),
}

impl<AccountId> Default for Owner<AccountId> {
	fn default() -> Self {
		Owner::None
	}
}

#[cfg_attr(feature = "std", derive(Debug))]
#[derive(Clone, Encode, Decode, PartialEq, Eq)]
pub struct PermissionsV1<AccountId> {
	pub update: Owner<AccountId>,
	pub mint: Owner<AccountId>,
	pub burn: Owner<AccountId>,
}

#[cfg_attr(feature = "std", derive(Debug))]
#[derive(Clone, Encode, Decode, PartialEq, Eq)]
pub enum PermissionVersions<AccountId> {
	V1(PermissionsV1<AccountId>),
}

/// Permissions Types in GenericAsset.
pub enum PermissionType {
	Burn,
	Mint,
	Update,
}

pub type PermissionLatest<AccountId> = PermissionsV1<AccountId>;

impl<AccountId> Default for PermissionVersions<AccountId> {
	fn default() -> Self {
		PermissionVersions::V1(Default::default())
	}
}

impl<AccountId> Default for PermissionsV1<AccountId> {
	fn default() -> Self {
		PermissionsV1 {
			update: Owner::None,
			mint: Owner::None,
			burn: Owner::None,
		}
	}
}

impl<AccountId> Into<PermissionLatest<AccountId>> for PermissionVersions<AccountId> {
	fn into(self) -> PermissionLatest<AccountId> {
		match self {
			PermissionVersions::V1(v1) => v1,
		}
	}
}

/// Converts the latest permission to other version.
impl<AccountId> Into<PermissionVersions<AccountId>> for PermissionLatest<AccountId> {
	fn into(self) -> PermissionVersions<AccountId> {
		PermissionVersions::V1(self)
	}
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		fn deposit_event<T>() = default;

		fn create(origin, options: AssetOptions<T::Balance, T::AccountId>) -> Result {

			let origin = ensure_signed(origin)?;
			let id = Self::next_asset_id();

			let permissions: PermissionVersions<T::AccountId> = options.permissions.clone().into();

			// The last available id serves as the overflow mark and won't be used.
			let next_id = id.checked_add(&One::one()).ok_or_else(||"No new assets id available.")?;

			// Force to reserve cennz.
			Self::reserve(&Self::staking_asset_id(), &origin, Self::create_asset_stake())?;

			<NextAssetId<T>>::put(next_id);
			<TotalIssuance<T>>::insert(id, &options.initial_issuance);
			<FreeBalance<T>>::insert(&id, &origin, options.initial_issuance);
			<Permissions<T>>::insert(&id, permissions);

			Self::deposit_event(RawEvent::Created(id, origin, options));

			Ok(())
		}

		/// Transfer some liquid free balance to another account.
		pub fn transfer(origin, #[compact] asset_id: T::AssetId, to: T::AccountId, #[compact] amount: T::Balance) {
			let origin = ensure_signed(origin)?;
			Self::make_transfer_with_fee(&asset_id, &origin, &to, amount)?;
		}

		/// Updates permission for a given asset_id and an account.
		/// The origin (account_id) should have `update` permission.
		fn update_permission(origin, #[compact] asset_id: T::AssetId, new_permission: PermissionLatest<T::AccountId>) -> Result {
			let origin = ensure_signed(origin)?;

			let permissions: PermissionVersions<T::AccountId> = new_permission.into();

			if Self::check_permission(&asset_id, &origin, &PermissionType::Update) {
				<Permissions<T>>::insert(asset_id, permissions);
				Ok(())
			} else {
				return Err("Origin does not have enough permission to update permissions.");
			}
		}

		/// Mints an asset, increases its amount.
		/// The origin should have `mint` permissions.
		fn mint(origin, #[compact] asset_id: T::AssetId, to: T::AccountId, amount: T::Balance) -> Result {
			let origin = ensure_signed(origin)?;
			if Self::check_permission(&asset_id, &origin, &PermissionType::Mint) {

				let original_free_balance = Self::free_balance(&asset_id, &to);
				let current_total_issuance = <TotalIssuance<T>>::get(asset_id);
				let new_total_issuance = current_total_issuance.checked_add(&amount).ok_or_else(|| "total_issuance got overflow after minting.")?;
				let value = original_free_balance.checked_add(&amount).ok_or_else(|| "free balance got overflow after minting.")?;

				<TotalIssuance<T>>::insert(asset_id, new_total_issuance);
				Self::set_free_balance(&asset_id, &to, value);

				Ok(())
			} else {
				return Err("The origin does not have permission to mint an asset, Permission error.");
			}
		}

		/// Burns an asset, decreases its amount.
		/// The origin should have `burn` permissions.
		fn burn(origin, #[compact] asset_id: T::AssetId, to: T::AccountId, amount: T::Balance) -> Result {
			let origin = ensure_signed(origin)?;

			if Self::check_permission(&asset_id, &origin, &PermissionType::Burn) {
				let original_free_balance = Self::free_balance(&asset_id, &to);

				let current_total_issuance = <TotalIssuance<T>>::get(asset_id);
				let new_total_issuance = current_total_issuance.checked_sub(&amount).ok_or_else(|| "total_issuance got underflow after burning")?;
				let value = original_free_balance.checked_sub(&amount).ok_or_else(|| "free_balance got underflow after burning")?;

				<TotalIssuance<T>>::insert(asset_id, new_total_issuance);

				Self::set_free_balance(&asset_id, &to, value);

				Ok(())
			} else {
				return Err("The origin does not have permission to burn an asset, Permission error.");
			}
		}

		/// Can be used to create reserved tokens.
		/// Requires Root call
		fn create_reserved(asset_id: T::AssetId, options: AssetOptions<T::Balance, T::AccountId>) -> Result {
			Self::create_asset(Some(asset_id), None, options)
		}
	}
}

#[derive(Encode, Decode, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "std", derive(Debug))]
pub struct BalanceLock<Balance, BlockNumber> {
	pub id: LockIdentifier,
	pub amount: Balance,
	pub until: BlockNumber,
	pub reasons: WithdrawReasons,
}

decl_storage! {
	trait Store for Module<T: Trait> as GenericAsset {
		/// Total issuance of a given asset.
		pub TotalIssuance get(total_issuance) build(|config: &GenesisConfig<T>| {
			let issuance = config.initial_balance * As::sa(config.endowed_accounts.len());
			config.assets.iter().map(|id| (id.clone(), issuance)).collect::<Vec<_>>()
		}): map T::AssetId => T::Balance;

		/// The free balance of a given asset under an account.
		pub FreeBalance: double_map T::AssetId, twox_128(T::AccountId) => T::Balance;

		/// The reserved balance of a given asset under an account.
		pub ReservedBalance: double_map T::AssetId, twox_128(T::AccountId) => T::Balance;

		/// Next available id for user created asset.
		pub NextAssetId get(next_asset_id) config(): T::AssetId;

		/// PermissionOptions for a given asset.
		pub Permissions get(get_permission): map T::AssetId => PermissionVersions<T::AccountId>;

		pub CreateAssetStakes get(create_asset_stake) config(): T::Balance;

		pub TransferFee get(transfer_fee) config(): T::Balance;

		/// Any liquidity locks on some account balances.
		pub Locks get(locks): map T::AccountId => Vec<BalanceLock<T::Balance, T::BlockNumber>>;

		/// Staking Asset Id
		pub StakingAssetId get(staking_asset_id) config(): T::AssetId;

		/// Spending Asset Id.
		pub SpendingAssetId get(spending_asset_id) config(): T::AssetId;
	}
	add_extra_genesis {
		config(assets): Vec<T::AssetId>;
		config(initial_balance): T::Balance;
		config(endowed_accounts): Vec<T::AccountId>;

		build(|storage: &mut primitives::StorageOverlay, _: &mut primitives::ChildrenStorageOverlay, config: &GenesisConfig<T>| {
			config.assets.iter().for_each(|asset_id| {
				config.endowed_accounts.iter().for_each(|account_id| {
					storage.insert(
						<FreeBalance<T>>::key_for(asset_id, account_id),
						<T::Balance as parity_codec::Encode>::encode(&config.initial_balance)
					);
				});
			});
		});
	}
}

decl_event!(
	pub enum Event<T> where
		<T as system::Trait>::AccountId,
		<T as Trait>::Balance,
		<T as Trait>::AssetId,
		AssetOptions = AssetOptions<<T as Trait>::Balance, <T as system::Trait>::AccountId>
	{
		/// Asset created (asset_id, creator, asset_options).
		Created(AssetId, AccountId, AssetOptions),
		/// Asset transfer succeeded (asset_id, from, to, amount).
		Transferred(AssetId, AccountId, AccountId, Balance),
		PermissionUpdated(AssetId, PermissionLatest<AccountId>),
		Minted(AssetId, AccountId, Balance),
		Burned(AssetId, AccountId, Balance),
	}
);

impl<T: Trait> Module<T> {
	// PUBLIC IMMUTABLES

	pub fn total_balance(asset_id: &T::AssetId, who: &T::AccountId) -> T::Balance {
		Self::free_balance(asset_id, who) + Self::reserved_balance(asset_id, who)
	}

	pub fn free_balance(asset_id: &T::AssetId, who: &T::AccountId) -> T::Balance {
		<FreeBalance<T>>::get(asset_id, who)
	}

	pub fn reserved_balance(asset_id: &T::AssetId, who: &T::AccountId) -> T::Balance {
		<ReservedBalance<T>>::get(asset_id, who)
	}

	/// Creates an asset.
	///
	/// # Arguments
	/// * `asset_id` An id of reserved asset. If not provided, an user generated asset would be created with next available id.
	/// * `from_account` An option value can have the account_id or None.
	/// * `asset_options` A struct which has the balance and permissions for the asset.
	///
	pub fn create_asset(
		asset_id: Option<T::AssetId>,
		from_account: Option<T::AccountId>,
		options: AssetOptions<T::Balance, T::AccountId>,
	) -> Result {
		let asset_id = if let Some(asset_id) = asset_id {
			ensure!(!<TotalIssuance<T>>::exists(&asset_id), "Asset id already taken.");
			ensure!(asset_id < Self::next_asset_id(), "Asset id not available.");
			asset_id
		} else {
			let asset_id = Self::next_asset_id();
			let next_id = asset_id
				.checked_add(&One::one())
				.ok_or_else(|| "No new user asset id available.")?;
			<NextAssetId<T>>::put(next_id);
			asset_id
		};

		let account_id = from_account.unwrap_or_else(Default::default);
		let permissions: PermissionVersions<T::AccountId> = options.permissions.clone().into();

		<TotalIssuance<T>>::insert(asset_id, &options.initial_issuance);
		<FreeBalance<T>>::insert(&asset_id, &account_id, options.initial_issuance);
		<Permissions<T>>::insert(&asset_id, permissions);

		Self::deposit_event(RawEvent::Created(asset_id, account_id, options));

		Ok(())
	}

	/// Transfer some liquid free balance from one account to another.
	/// This will not charge transfer fee and will not emit Transferred event.
	pub fn make_transfer(asset_id: &T::AssetId, from: &T::AccountId, to: &T::AccountId, amount: T::Balance) -> Result {
		let from_balance = Self::free_balance(asset_id, from);
		ensure!(from_balance >= amount, "balance too low to send amount");

		if from != to {
			<FreeBalance<T>>::mutate(asset_id, from, |balance| *balance -= amount);
			<FreeBalance<T>>::mutate(asset_id, to, |balance| *balance += amount);
		}

		Ok(())
	}

	/// Transfer some liquid free balance from one account to another.
	/// This will charge transfer fee and will emit Transferred event.
	pub fn make_transfer_with_fee(
		asset_id: &T::AssetId,
		from: &T::AccountId,
		to: &T::AccountId,
		amount: T::Balance,
	) -> Result {
		ensure!(!amount.is_zero(), "cannot transfer zero amount");

		let from_balance = Self::free_balance(asset_id, from);
		let total_amount = amount
			.checked_add(&Self::transfer_fee())
			.ok_or_else(|| "transfer amount plus fee overflow")?;
		ensure!(from_balance >= total_amount, "balance too low to send amount");

		if from != to {
			T::ChargeFee::charge_fee(from, Self::transfer_fee())?;

			<FreeBalance<T>>::mutate(asset_id, from, |balance| *balance -= amount);
			<FreeBalance<T>>::mutate(asset_id, to, |balance| *balance += amount);

			Self::deposit_event(RawEvent::Transferred(*asset_id, from.clone(), to.clone(), amount));
		}

		Ok(())
	}

	/// Moves `amount` from balance to reserved balance.
	///
	/// If the free balance is lower than `amount`, then no funds will be moved and an `Err` will
	/// be returned to notify of this. This is different behaviour to `unreserve`.
	pub fn reserve(asset_id: &T::AssetId, who: &T::AccountId, amount: T::Balance) -> Result {
		// Do we need to consider that this is an atomic transaction?
		let original_reserve_balance = Self::reserved_balance(asset_id, who);
		let original_free_balance = Self::free_balance(asset_id, who);
		if original_free_balance < amount {
			return Err("not enough free funds");
		}
		let new_reserve_balance = original_reserve_balance + amount;
		Self::set_reserved_balance(asset_id, who, new_reserve_balance);
		let new_free_balance = original_free_balance - amount;
		Self::set_free_balance(asset_id, who, new_free_balance);
		Ok(())
	}

	/// Moves up to `amount` from reserved balance to balance. This function cannot fail.
	///
	/// As much funds up to `amount` will be deducted as possible, `remaining` will be returned.
	/// NOTE: This is different to `reserve`.
	pub fn unreserve(asset_id: &T::AssetId, who: &T::AccountId, amount: T::Balance) -> T::Balance {
		let b = Self::reserved_balance(asset_id, who);
		let actual = rstd::cmp::min(b, amount);
		let original_free_balance = Self::free_balance(asset_id, who);
		let new_free_balance = original_free_balance + actual;
		Self::set_free_balance(asset_id, who, new_free_balance);
		Self::set_reserved_balance(asset_id, who, b - actual);
		amount - actual
	}

	/// Deducts up to `amount` from the combined balance of `who`, preferring to deduct from the
	/// free balance. This function cannot fail.
	///
	/// As much funds up to `amount` will be deducted as possible. If this is less than `amount`,
	/// then `Some(remaining)` will be returned. Full completion is given by `None`.
	pub fn slash(asset_id: &T::AssetId, who: &T::AccountId, amount: T::Balance) -> Option<T::Balance> {
		let free_balance = Self::free_balance(asset_id, who);
		let free_slash = rstd::cmp::min(free_balance, amount);
		let new_free_balance = free_balance - free_slash;
		Self::set_free_balance(asset_id, who, new_free_balance);
		// TODO: implement staking here
		// Self::decrease_total_stake_by(free_slash);
		// Question: are we slashing reserved in this case?
		if free_slash < amount {
			Self::slash_reserved(asset_id, who, amount - free_slash)
		} else {
			None
		}
	}

	/// Adds up to `amount` to the free balance of `who`.
	///
	/// If `who` doesn't exist, nothing is done and an Err returned.
	pub fn reward(asset_id: &T::AssetId, who: &T::AccountId, amount: T::Balance) -> Result {
		let original_free_balance = Self::free_balance(asset_id, who);
		let new_free_balance = original_free_balance + amount;
		Self::set_free_balance(asset_id, who, new_free_balance);
		// TODO: implement staking here
		// Self::increase_total_stake_by(amount);
		Ok(())
	}

	/// Deducts up to `amount` from reserved balance of `who`. This function cannot fail.
	///
	/// As much funds up to `amount` will be deducted as possible. If this is less than `amount`,
	/// then `Some(remaining)` will be returned. Full completion is given by `None`.
	pub fn slash_reserved(asset_id: &T::AssetId, who: &T::AccountId, amount: T::Balance) -> Option<T::Balance> {
		let original_reserve_balance = Self::reserved_balance(asset_id, who);
		let slash = rstd::cmp::min(original_reserve_balance, amount);
		let new_reserve_balance = original_reserve_balance - slash;
		Self::set_reserved_balance(asset_id, who, new_reserve_balance);
		// TODO: implement staking here
		// Self::decrease_total_stake_by(slash);
		if amount == slash {
			None
		} else {
			Some(amount - slash)
		}
	}

	/// Moves up to `amount` from reserved balance of account `who` to free balance of account
	/// `beneficiary`. `beneficiary` must exist for this to succeed. If it does not, `Err` will be
	/// returned.
	///
	/// As much funds up to `amount` will be moved as possible. If this is less than `amount`, then
	/// the `remaining` would be returned, else `Zero::zero()`.
	pub fn repatriate_reserved(
		asset_id: &T::AssetId,
		who: &T::AccountId,
		beneficiary: &T::AccountId,
		amount: T::Balance,
	) -> rstd::result::Result<T::Balance, &'static str> {
		let b = Self::reserved_balance(asset_id, who);
		let slash = rstd::cmp::min(b, amount);

		let original_free_balance = Self::free_balance(asset_id, beneficiary);
		let new_free_balance = original_free_balance + slash;
		Self::set_free_balance(asset_id, beneficiary, new_free_balance);

		let new_reserve_balance = b - slash;
		Self::set_reserved_balance(asset_id, who, new_reserve_balance);
		Ok(amount - slash)
	}

	/// Checks permission to perfrom burn, mint or update
	///
	/// # Arguments
	/// * `asset_id` -  A T::AssetId type contains the asset_id which the permission embedded.
	/// * `who` - A T::AccountId type contains the account_id that the permission going to check.
	/// * `what` - A string slice contains the permission type.
	///
	pub fn check_permission(asset_id: &T::AssetId, who: &T::AccountId, what: &PermissionType) -> bool {
		let permission_versions: PermissionVersions<T::AccountId> = Self::get_permission(asset_id); // This returns an enum.
		let permission = permission_versions.into();

		match (what, permission) {
			(
				PermissionType::Burn,
				PermissionLatest {
					burn: Owner::Address(account),
					..
				},
			) => account == *who,
			(
				PermissionType::Mint,
				PermissionLatest {
					mint: Owner::Address(account),
					..
				},
			) => account == *who,
			(
				PermissionType::Update,
				PermissionLatest {
					update: Owner::Address(account),
					..
				},
			) => account == *who,
			_ => false,
		}
	}

	/// Returns `Ok` iff the account is able to make a withdrawal of the given amount
	/// for the given reason.
	///
	/// `Err(...)` with the reason why not otherwise.
	pub fn ensure_can_withdraw(
		asset_id: &T::AssetId,
		who: &T::AccountId,
		_amount: T::Balance,
		reason: WithdrawReason,
		new_balance: T::Balance,
	) -> Result {
		if asset_id != &Self::staking_asset_id() {
			return Ok(());
		}

		let locks = Self::locks(who);
		if locks.is_empty() {
			return Ok(());
		}
		let now = <system::Module<T>>::block_number();
		if Self::locks(who)
			.into_iter()
			.all(|l| now >= l.until || new_balance >= l.amount || !l.reasons.contains(reason))
		{
			Ok(())
		} else {
			Err("account liquidity restrictions prevent withdrawal")
		}
	}

	// PRIVATE MUTABLES

	/// NOTE: LOW-LEVEL: This will not attempt to maintain total issuance. It is expected that
	/// the caller will do this.
	fn set_reserved_balance(asset_id: &T::AssetId, who: &T::AccountId, balance: T::Balance) {
		<ReservedBalance<T>>::insert(asset_id, who, balance);
	}

	/// NOTE: LOW-LEVEL: This will not attempt to maintain total issuance. It is expected that
	/// the caller will do this.
	fn set_free_balance(asset_id: &T::AssetId, who: &T::AccountId, balance: T::Balance) {
		<FreeBalance<T>>::insert(asset_id, who, balance);
	}

	fn set_lock(
		id: LockIdentifier,
		who: &T::AccountId,
		amount: T::Balance,
		until: T::BlockNumber,
		reasons: WithdrawReasons,
	) {
		let now = <system::Module<T>>::block_number();
		let mut new_lock = Some(BalanceLock {
			id,
			amount,
			until,
			reasons,
		});
		let mut locks = <Module<T>>::locks(who)
			.into_iter()
			.filter_map(|l| {
				if l.id == id {
					new_lock.take()
				} else if l.until > now {
					Some(l)
				} else {
					None
				}
			})
			.collect::<Vec<_>>();
		if let Some(lock) = new_lock {
			locks.push(lock)
		}
		<Locks<T>>::insert(who, locks);
	}

	fn extend_lock(
		id: LockIdentifier,
		who: &T::AccountId,
		amount: T::Balance,
		until: T::BlockNumber,
		reasons: WithdrawReasons,
	) {
		let now = <system::Module<T>>::block_number();
		let mut new_lock = Some(BalanceLock {
			id,
			amount,
			until,
			reasons,
		});
		let mut locks = <Module<T>>::locks(who)
			.into_iter()
			.filter_map(|l| {
				if l.id == id {
					new_lock.take().map(|nl| BalanceLock {
						id: l.id,
						amount: l.amount.max(nl.amount),
						until: l.until.max(nl.until),
						reasons: l.reasons | nl.reasons,
					})
				} else if l.until > now {
					Some(l)
				} else {
					None
				}
			})
			.collect::<Vec<_>>();
		if let Some(lock) = new_lock {
			locks.push(lock)
		}
		<Locks<T>>::insert(who, locks);
	}

	fn remove_lock(id: LockIdentifier, who: &T::AccountId) {
		let now = <system::Module<T>>::block_number();
		let locks = <Module<T>>::locks(who)
			.into_iter()
			.filter_map(|l| if l.until > now && l.id != id { Some(l) } else { None })
			.collect::<Vec<_>>();
		<Locks<T>>::insert(who, locks);
	}
}

pub trait AssetIdProvider {
	type AssetId;
	fn asset_id() -> Self::AssetId;
	fn reward_asset_id() -> Self::AssetId {
		Self::asset_id()
	}
}

// wrapping these imbalanes in a private module is necessary to ensure absolute privacy
// of the inner member.
mod imbalances {
	use super::{result, AssetIdProvider, Imbalance, Saturating, StorageMap, Subtrait, Zero};
	use rstd::mem;

	/// Opaque, move-only struct with private fields that serves as a token denoting that
	/// funds have been created without any equal and opposite accounting.
	#[must_use]
	pub struct PositiveImbalance<T: Subtrait, U: AssetIdProvider<AssetId = T::AssetId>>(
		T::Balance,
		rstd::marker::PhantomData<U>,
	);
	impl<T, U> PositiveImbalance<T, U>
	where
		T: Subtrait,
		U: AssetIdProvider<AssetId = T::AssetId>,
	{
		pub fn new(amount: T::Balance) -> Self {
			PositiveImbalance(amount, Default::default())
		}
	}

	/// Opaque, move-only struct with private fields that serves as a token denoting that
	/// funds have been destroyed without any equal and opposite accounting.
	#[must_use]
	pub struct NegativeImbalance<T: Subtrait, U: AssetIdProvider<AssetId = T::AssetId>>(
		T::Balance,
		rstd::marker::PhantomData<U>,
	);
	impl<T, U> NegativeImbalance<T, U>
	where
		T: Subtrait,
		U: AssetIdProvider<AssetId = T::AssetId>,
	{
		pub fn new(amount: T::Balance) -> Self {
			NegativeImbalance(amount, Default::default())
		}
	}

	impl<T, U> Imbalance<T::Balance> for PositiveImbalance<T, U>
	where
		T: Subtrait,
		U: AssetIdProvider<AssetId = T::AssetId>,
	{
		type Opposite = NegativeImbalance<T, U>;

		fn zero() -> Self {
			Self::new(Zero::zero())
		}
		fn drop_zero(self) -> result::Result<(), Self> {
			if self.0.is_zero() {
				Ok(())
			} else {
				Err(self)
			}
		}
		fn split(self, amount: T::Balance) -> (Self, Self) {
			let first = self.0.min(amount);
			let second = self.0 - first;

			mem::forget(self);
			(Self::new(first), Self::new(second))
		}
		fn merge(mut self, other: Self) -> Self {
			self.0 = self.0.saturating_add(other.0);
			mem::forget(other);

			self
		}
		fn subsume(&mut self, other: Self) {
			self.0 = self.0.saturating_add(other.0);
			mem::forget(other);
		}
		fn offset(self, other: Self::Opposite) -> result::Result<Self, Self::Opposite> {
			let (a, b) = (self.0, other.0);
			mem::forget((self, other));

			if a >= b {
				Ok(Self::new(a - b))
			} else {
				Err(NegativeImbalance::new(b - a))
			}
		}
		fn peek(&self) -> T::Balance {
			self.0.clone()
		}
	}

	impl<T, U> Imbalance<T::Balance> for NegativeImbalance<T, U>
	where
		T: Subtrait,
		U: AssetIdProvider<AssetId = T::AssetId>,
	{
		type Opposite = PositiveImbalance<T, U>;

		fn zero() -> Self {
			Self::new(Zero::zero())
		}
		fn drop_zero(self) -> result::Result<(), Self> {
			if self.0.is_zero() {
				Ok(())
			} else {
				Err(self)
			}
		}
		fn split(self, amount: T::Balance) -> (Self, Self) {
			let first = self.0.min(amount);
			let second = self.0 - first;

			mem::forget(self);
			(Self::new(first), Self::new(second))
		}
		fn merge(mut self, other: Self) -> Self {
			self.0 = self.0.saturating_add(other.0);
			mem::forget(other);

			self
		}
		fn subsume(&mut self, other: Self) {
			self.0 = self.0.saturating_add(other.0);
			mem::forget(other);
		}
		fn offset(self, other: Self::Opposite) -> result::Result<Self, Self::Opposite> {
			let (a, b) = (self.0, other.0);
			mem::forget((self, other));

			if a >= b {
				Ok(Self::new(a - b))
			} else {
				Err(PositiveImbalance::new(b - a))
			}
		}
		fn peek(&self) -> T::Balance {
			self.0.clone()
		}
	}

	impl<T, U> Drop for PositiveImbalance<T, U>
	where
		T: Subtrait,
		U: AssetIdProvider<AssetId = T::AssetId>,
	{
		/// Basic drop handler will just square up the total issuance.
		fn drop(&mut self) {
			<super::TotalIssuance<super::ElevatedTrait<T>>>::mutate(&U::asset_id(), |v| *v = v.saturating_add(self.0));
		}
	}

	impl<T, U> Drop for NegativeImbalance<T, U>
	where
		T: Subtrait,
		U: AssetIdProvider<AssetId = T::AssetId>,
	{
		/// Basic drop handler will just square up the total issuance.
		fn drop(&mut self) {
			<super::TotalIssuance<super::ElevatedTrait<T>>>::mutate(&U::asset_id(), |v| *v = v.saturating_sub(self.0));
		}
	}
}

// TODO: #2052
// Somewhat ugly hack in order to gain access to module's `increase_total_issuance_by`
// using only the Subtrait (which defines only the types that are not dependent
// on Positive/NegativeImbalance). Subtrait must be used otherwise we end up with a
// circular dependency with Trait having some types be dependent on PositiveImbalance<Trait>
// and PositiveImbalance itself depending back on Trait for its Drop impl (and thus
// its type declaration).
// This works as long as `increase_total_issuance_by` doesn't use the Imbalance
// types (basically for charging fees).
// This should eventually be refactored so that the three type items that do
// depend on the Imbalance type (TransactionPayment, TransferPayment, DustRemoval)
// are placed in their own SRML module.
struct ElevatedTrait<T: Subtrait>(T);
impl<T: Subtrait> Clone for ElevatedTrait<T> {
	fn clone(&self) -> Self {
		unimplemented!()
	}
}
impl<T: Subtrait> PartialEq for ElevatedTrait<T> {
	fn eq(&self, _: &Self) -> bool {
		unimplemented!()
	}
}
impl<T: Subtrait> Eq for ElevatedTrait<T> {}
impl<T: Subtrait> system::Trait for ElevatedTrait<T> {
	type Origin = T::Origin;
	type Index = T::Index;
	type BlockNumber = T::BlockNumber;
	type Hash = T::Hash;
	type Hashing = T::Hashing;
	type Digest = T::Digest;
	type AccountId = T::AccountId;
	type Lookup = T::Lookup;
	type Header = T::Header;
	type Event = ();
	type Log = T::Log;
}
impl<T: Subtrait> Trait for ElevatedTrait<T> {
	type Balance = T::Balance;
	type AssetId = T::AssetId;
	type ChargeFee = DummyChargeFee<T::AccountId, T::Balance>;
	type Event = ();
}

#[derive(Encode, Decode, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "std", derive(Debug))]
pub struct AssetCurrency<T, U>(rstd::marker::PhantomData<T>, rstd::marker::PhantomData<U>);

impl<T, U> Currency<T::AccountId> for AssetCurrency<T, U>
where
	T: Trait,
	U: AssetIdProvider<AssetId = T::AssetId>,
{
	type Balance = T::Balance;
	type PositiveImbalance = PositiveImbalance<T, U>;
	type NegativeImbalance = NegativeImbalance<T, U>;

	fn total_balance(who: &T::AccountId) -> Self::Balance {
		Self::free_balance(&who) + Self::reserved_balance(&who)
	}

	fn free_balance(who: &T::AccountId) -> Self::Balance {
		<Module<T>>::free_balance(&U::asset_id(), &who)
	}

	/// Returns the total staking asset issuance
	fn total_issuance() -> Self::Balance {
		<Module<T>>::total_issuance(U::asset_id())
	}

	fn minimum_balance() -> Self::Balance {
		Zero::zero()
	}

	fn transfer(transactor: &T::AccountId, dest: &T::AccountId, value: Self::Balance) -> Result {
		<Module<T>>::make_transfer(&U::asset_id(), transactor, dest, value)
	}

	fn ensure_can_withdraw(
		who: &T::AccountId,
		amount: Self::Balance,
		reason: WithdrawReason,
		new_balance: Self::Balance,
	) -> Result {
		<Module<T>>::ensure_can_withdraw(&U::asset_id(), who, amount, reason, new_balance)
	}

	fn withdraw(
		who: &T::AccountId,
		value: Self::Balance,
		reason: WithdrawReason,
		_: ExistenceRequirement, // no existential deposit policy for generic asset
	) -> result::Result<Self::NegativeImbalance, &'static str> {
		let new_balance = <Module<T>>::free_balance(&U::asset_id(), who)
			.checked_sub(&value)
			.ok_or_else(|| "account has too few funds")?;
		Self::ensure_can_withdraw(who, value, reason, new_balance)?;
		<Module<T>>::set_free_balance(&U::asset_id(), who, new_balance);
		Ok(NegativeImbalance::new(value))
	}

	fn deposit_into_existing(
		who: &T::AccountId,
		value: Self::Balance,
	) -> result::Result<Self::PositiveImbalance, &'static str> {
		<Module<T>>::set_free_balance(&U::asset_id(), who, Self::free_balance(who) + value);
		Ok(PositiveImbalance::new(value))
	}

	fn deposit_creating(who: &T::AccountId, value: Self::Balance) -> Self::PositiveImbalance {
		let (imbalance, _) = Self::make_free_balance_be(who, Self::free_balance(who) + value);
		if let SignedImbalance::Positive(p) = imbalance {
			p
		} else {
			// Impossible, but be defensive.
			Self::PositiveImbalance::zero()
		}
	}

	fn make_free_balance_be(
		who: &T::AccountId,
		balance: Self::Balance,
	) -> (
		SignedImbalance<Self::Balance, Self::PositiveImbalance>,
		UpdateBalanceOutcome,
	) {
		let original = <Module<T>>::free_balance(&U::asset_id(), who);
		let imbalance = if original <= balance {
			SignedImbalance::Positive(PositiveImbalance::new(balance - original))
		} else {
			SignedImbalance::Negative(NegativeImbalance::new(original - balance))
		};
		<Module<T>>::set_free_balance(&U::asset_id(), who, balance);
		(imbalance, UpdateBalanceOutcome::Updated)
	}

	fn can_slash(who: &T::AccountId, value: Self::Balance) -> bool {
		<Module<T>>::free_balance(&U::asset_id(), &who) >= value
	}

	fn slash(who: &T::AccountId, value: Self::Balance) -> (Self::NegativeImbalance, Self::Balance) {
		let remaining = <Module<T>>::slash(&U::asset_id(), who, value);
		if let Some(r) = remaining {
			(NegativeImbalance::new(value - r), r)
		} else {
			(NegativeImbalance::new(value), Zero::zero())
		}
	}
}

impl<T, U> ReservableCurrency<T::AccountId> for AssetCurrency<T, U>
where
	T: Trait,
	U: AssetIdProvider<AssetId = T::AssetId>,
{
	fn can_reserve(who: &T::AccountId, value: Self::Balance) -> bool {
		// TODO: check with lock
		<Module<T>>::free_balance(&U::asset_id(), &who) >= value
	}

	fn reserved_balance(who: &T::AccountId) -> Self::Balance {
		<Module<T>>::reserved_balance(&U::asset_id(), &who)
	}

	fn reserve(who: &T::AccountId, value: Self::Balance) -> result::Result<(), &'static str> {
		<Module<T>>::reserve(&U::reward_asset_id(), who, value)
	}

	fn unreserve(who: &T::AccountId, value: Self::Balance) -> Self::Balance {
		<Module<T>>::unreserve(&U::asset_id(), who, value)
	}

	fn slash_reserved(who: &T::AccountId, value: Self::Balance) -> (Self::NegativeImbalance, Self::Balance) {
		let b = Self::reserved_balance(&who.clone());
		let slash = cmp::min(b, value);

		<Module<T>>::set_reserved_balance(&U::asset_id(), who, b - slash);
		(NegativeImbalance::new(slash), value - slash)
	}

	fn repatriate_reserved(
		slashed: &T::AccountId,
		beneficiary: &T::AccountId,
		value: Self::Balance,
	) -> result::Result<Self::Balance, &'static str> {
		<Module<T>>::repatriate_reserved(&U::asset_id(), slashed, beneficiary, value)
	}
}

pub struct StakingAssetIdProvider<T>(rstd::marker::PhantomData<T>);

impl<T: Trait> AssetIdProvider for StakingAssetIdProvider<T> {
	type AssetId = T::AssetId;
	fn asset_id() -> Self::AssetId {
		<Module<T>>::staking_asset_id()
	}
}

pub struct SpendingAssetIdProvider<T>(rstd::marker::PhantomData<T>);

impl<T: Trait> AssetIdProvider for SpendingAssetIdProvider<T> {
	type AssetId = T::AssetId;
	fn asset_id() -> Self::AssetId {
		<Module<T>>::spending_asset_id()
	}
}

/// STAKE for balance, SPEND for reward
pub struct RewardAssetIdProvider<T>(rstd::marker::PhantomData<T>);

impl<T: Trait> AssetIdProvider for RewardAssetIdProvider<T> {
	type AssetId = T::AssetId;
	fn asset_id() -> Self::AssetId {
		<Module<T>>::staking_asset_id()
	}
	fn reward_asset_id() -> Self::AssetId {
		<Module<T>>::spending_asset_id()
	}
}

impl<T> LockableCurrency<T::AccountId> for AssetCurrency<T, StakingAssetIdProvider<T>>
where
	T: Trait,
	T::Balance: MaybeSerializeDebug,
{
	type Moment = T::BlockNumber;

	fn set_lock(
		id: LockIdentifier,
		who: &T::AccountId,
		amount: T::Balance,
		until: T::BlockNumber,
		reasons: WithdrawReasons,
	) {
		<Module<T>>::set_lock(id, who, amount, until, reasons)
	}

	fn extend_lock(
		id: LockIdentifier,
		who: &T::AccountId,
		amount: T::Balance,
		until: T::BlockNumber,
		reasons: WithdrawReasons,
	) {
		<Module<T>>::extend_lock(id, who, amount, until, reasons)
	}

	fn remove_lock(id: LockIdentifier, who: &T::AccountId) {
		<Module<T>>::remove_lock(id, who)
	}
}

impl<T> LockableCurrency<T::AccountId> for AssetCurrency<T, RewardAssetIdProvider<T>>
where
	T: Trait,
	T::Balance: MaybeSerializeDebug,
{
	type Moment = T::BlockNumber;

	fn set_lock(
		id: LockIdentifier,
		who: &T::AccountId,
		amount: T::Balance,
		until: T::BlockNumber,
		reasons: WithdrawReasons,
	) {
		<Module<T>>::set_lock(id, who, amount, until, reasons)
	}

	fn extend_lock(
		id: LockIdentifier,
		who: &T::AccountId,
		amount: T::Balance,
		until: T::BlockNumber,
		reasons: WithdrawReasons,
	) {
		<Module<T>>::extend_lock(id, who, amount, until, reasons)
	}

	fn remove_lock(id: LockIdentifier, who: &T::AccountId) {
		<Module<T>>::remove_lock(id, who)
	}
}

pub type StakingAssetCurrency<T> = AssetCurrency<T, StakingAssetIdProvider<T>>;
pub type SpendingAssetCurrency<T> = AssetCurrency<T, SpendingAssetIdProvider<T>>;
pub type RewardAssetCurrency<T> = AssetCurrency<T, RewardAssetIdProvider<T>>;