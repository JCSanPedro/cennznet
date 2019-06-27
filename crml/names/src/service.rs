// Copyright (C) 2019 Centrality Investments Limited
// This file is part of CENNZnet.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.
use srml_support::{dispatch::Result, dispatch::Vec, StorageMap};
use {system::ensure_signed};
extern crate srml_system as system;

extern crate sr_io;
extern crate runtime_primitives;
extern crate primitives;

pub type Name = Vec<u8>;
// pub struct NameSubscription = {
//     name: Name,
//     expiration: u32
// }

// #[derive(Encode, Decode, Clone, Eq, PartialEq)]
// #[cfg_attr(feature = "std", derive(Debug))]
// pub struct NameSubscription {
//
// }

pub trait Trait: system::Trait {

}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		// fn deposit_event<T>() = default;

        fn create(origin, name: Name) {
            let user = ensure_signed(origin)?;
            ensure!(!<Address<T>>::exists(&name), "This name is reserved");
            <Address<T>>::insert(name.clone(), user.clone());
            let mut names = <Names<T>>::get(&user);
            names.push(name);
            <Names<T>>::insert(&user, names);
        }

        fn update(origin, name: Name, new_address: T::AccountId) {
            let user = ensure_signed(origin)?;
            let owner = <Address<T>>::get(&name);
            ensure!(user.clone() == owner.clone(), "User does not own name");
            ensure!(<Address<T>>::exists(&name), "This name does not exist");
            <Address<T>>::remove(name.clone());
            <Address<T>>::insert(name.clone(), new_address.clone());
            let mut names = <Names<T>>::get(&new_address);
            names.push(name);
            <Names<T>>::insert(&new_address, names);
        }

        fn delete(origin, name: Name) {
            let user = ensure_signed(origin)?;
            let owner = <Address<T>>::get(&name);
            ensure!(user.clone() == owner.clone(), "User does not own name");
            <Address<T>>::remove(name.clone());
            let mut names= <Names<T>>::get(&user);
            names = names
                .into_iter()
                .filter(|existing_name| &name != existing_name)
                .collect();
            <Names<T>>::insert(&user, names);
        }

        fn renew(origin, _name: Name) {
            let user = ensure_signed(origin)?;
        }
	}
}

// The data that is stored
decl_storage! {
	trait Store for Module<T: Trait> as NamesService {
		pub Address get(address): map Name => T::AccountId;
        pub Names get(names): map T::AccountId => Vec<Name>;
	}
}


impl<T: Trait> Module<T> {
    fn resolve_address(name: Name) -> Option<T::AccountId> {
        None
    }
}

#[cfg(test)]
mod tests {
	use super::*;

	use codec::{Decode, Encode};
	use serde::{Deserialize, Serialize};
	use runtime_primitives::traits::{Verify, Lazy};

	use self::sr_io::with_externalities;
	use self::primitives::{Blake2Hasher, H256};
	// The testing primitives are very useful for avoiding having to work with signatures
	// or public keys. `u64` is used as the `AccountId` and no `Signature`s are requried.
	use self::runtime_primitives::{
		testing::{Digest, DigestItem, Header},
		traits::{BlakeTwo256, IdentityLookup},
		BuildStorage,
	};

	impl_outer_origin! {
		pub enum Origin for Test {}
	}

	#[derive(Encode, Decode, Serialize, Deserialize, Debug)]
	pub struct Signature;

	impl Verify for Signature {
		type Signer = H256;
		fn verify<L: Lazy<[u8]>>(&self, _msg: L, _signer: &Self::Signer) -> bool {
			true
		}
	}

	// For testing the module, we construct most of a mock runtime. This means
	// first constructing a configuration type (`Test`) which `impl`s each of the
	// configuration traits of modules we want to use.
	#[derive(Clone, Eq, PartialEq)]
	pub struct Test;
	impl system::Trait for Test {
		type Origin = Origin;
		type Index = u64;
		type BlockNumber = u64;
		type Hash = H256;
		type Hashing = BlakeTwo256;
		type Digest = Digest;
		type AccountId = H256;
		type Lookup = IdentityLookup<H256>;
		type Header = Header;
		type Event = ();
		type Log = DigestItem;
		type Signature = Signature;
	}
	impl Trait for Test {}
	type Names = Module<Test>;

	// This function basically just builds a genesis storage key/value store according to
	// our desired mockup.
	fn new_test_ext() -> sr_io::TestExternalities<Blake2Hasher> {
		system::GenesisConfig::<Test>::default()
			.build_storage()
			.unwrap()
			.0
			.into()
	}

	#[test]
	fn should_create_name() {
		with_externalities(&mut new_test_ext(), || {
			let name = b"SuperName".to_vec();

			assert_ok!(Names::create(
				Origin::signed(H256::from_low_u64_be(1)),
				name.clone()
			));

            assert_eq!(
				Names::names(H256::from_low_u64_be(1)),
				vec![name.clone()]
			);
	    })
    }

	#[test]
	fn should_update_name() {
		with_externalities(&mut new_test_ext(), || {
			let name = b"SuperName".to_vec();

			assert_ok!(Names::create(
				Origin::signed(H256::from_low_u64_be(1)),
				name.clone()
			));

			assert_ok!(Names::update(
				Origin::signed(H256::from_low_u64_be(1)),
				name.clone(),
				H256::from_low_u64_be(2),
			));

            assert_eq!(
				Names::names(H256::from_low_u64_be(2)),
				vec![name.clone()]
			);
	    })
    }
}
