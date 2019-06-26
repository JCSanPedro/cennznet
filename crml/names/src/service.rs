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
use {system::ensure_signed}
extern crate srml_system as system;

pub type Name = Vec<u8>;
pub struct NameSubscription = {
    name: Name,
    expiration: u32
}

// #[derive(Encode, Decode, Clone, Eq, PartialEq)]
// #[cfg_attr(feature = "std", derive(Debug))]
// pub struct NameSubscription {
//
// }

pub trait Trait: system::Trait {
	/// The overarching event type.
	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		fn deposit_event<T>() = default;

        fn create(origin, name: Name, subscriptionTime: u32) {
            let user = ensure_signed(origin)?;

            Ok(())
        }

        fn update(origin, name: Name, newAddress: T::AccountId) {
            let user = ensure_signed(origin)?;

            Ok(())
        }

        fn delete(origin, name: Name) {
            let user = ensure_signed(origin)?;

            Ok(())
        }

        fn renew(origin, name: Name, subscriptionTime: u32) {
            let user = ensure_signed(origin)?;

            Ok(())
        }
	}
}

// The data that is stored
decl_storage! {
	trait Store for Module<T: Trait> as NamesService {
		pub Address get(address): map Name => T::AccountId;
        pub Names get(names): map T::AccountId => vec<Name>;
	}
}

decl_event!(
	pub enum Event<T> where <T as system::Trait>::Hash, <T as system::Trait>::AccountId {
		NameAdded(AccountId, Name),
	}
);

impl<T: Trait> Module<T> {
    fn resolve_address(name: Name) -> Option<T::AccountId> {
        <Address<T>>::get(name)
    }
}
