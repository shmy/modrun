use std::any::{TypeId, type_name};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::group::Group;
use crate::provide::DynProvider;
use crate::scope::ScopeId;

use super::Container;
use super::storage::{pack, take_packed_member};
use super::types::{ConstructOut, Constructed, GroupElementKey, GroupRegistration, ProviderKey};

impl Container {
    pub(crate) fn ensure_empty_group<T: Clone + Send + Sync + 'static>(&mut self) -> Result<()> {
        let element = TypeId::of::<T>();
        let group_type = TypeId::of::<Group<T>>();
        self.ensure_group_virtual::<T>(element, group_type)
    }

    pub(crate) fn insert_group_member_typed<T: Clone + Send + Sync + 'static>(
        &mut self,
        provider: DynProvider,
        scope: ScopeId,
    ) -> Result<()> {
        let element = TypeId::of::<T>();
        let group_type = TypeId::of::<Group<T>>();
        self.ensure_group_virtual::<T>(element, group_type)?;

        self.next_group_member_id = self
            .next_group_member_id
            .checked_add(1)
            .expect("group member id overflow");
        let key = ProviderKey {
            type_id: provider.result_type(),
            scope,
            private: false,
            ordinal: self.next_group_member_id,
        };

        self.providers.insert(key, provider);
        self.provider_order_index
            .insert(key, self.provider_order.len());
        self.provider_order.push(key);
        self.group_members
            .entry(GroupElementKey { element })
            .or_default()
            .push(key);
        Ok(())
    }

    pub(crate) fn require_group_element(&mut self, element: TypeId, type_name: &'static str) {
        self.required_groups.entry(element).or_insert(type_name);
    }

    pub(crate) fn group_element_type(&self, group_type: TypeId) -> Option<TypeId> {
        self.group_by_type.get(&group_type).copied()
    }

    pub(crate) fn is_group_type(&self, id: TypeId) -> bool {
        self.group_by_type.contains_key(&id)
    }

    pub(crate) fn group_virtual_key(&self, group_type: TypeId) -> Option<ProviderKey> {
        let element = self.group_by_type.get(&group_type)?;
        self.group_registrations
            .get(element)
            .map(|reg| reg.virtual_key)
    }

    pub(crate) fn group_members_for_element(&self, element: TypeId) -> Option<&[ProviderKey]> {
        self.group_members
            .get(&GroupElementKey { element })
            .map(Vec::as_slice)
    }

    pub(crate) fn store_group_member(&mut self, key: ProviderKey, value: super::DynAny) {
        self.member_values.insert(key, value);
    }

    pub(crate) fn take_group_member(&mut self, key: ProviderKey) -> Option<super::DynAny> {
        self.member_values.remove(&key)
    }

    fn ensure_group_virtual<T: Clone + Send + Sync + 'static>(
        &mut self,
        element: TypeId,
        group_type: TypeId,
    ) -> Result<()> {
        if self.group_registrations.contains_key(&element) {
            return Ok(());
        }

        let arc_type = TypeId::of::<Arc<Group<T>>>();
        super::storage::ensure_absent(
            self,
            group_type,
            type_name::<Group<T>>(),
            ScopeId::ROOT,
            false,
        )?;
        super::storage::ensure_absent(
            self,
            arc_type,
            type_name::<Arc<Group<T>>>(),
            ScopeId::ROOT,
            false,
        )?;

        let virtual_key = ProviderKey::singleton(group_type, ScopeId::ROOT, false);
        let construct = Box::new(move |container: &mut Container| -> Result<ConstructOut> {
            let built = aggregate_group::<T>(container, element, type_name::<Group<T>>())?;
            Ok(ConstructOut::Ready(built))
        });
        let provider = DynProvider::new_group_virtual::<T>(construct);
        self.providers.insert(virtual_key, provider);
        self.provider_order_index
            .insert(virtual_key, self.provider_order.len());
        self.provider_order.push(virtual_key);
        self.public_index.insert(group_type, virtual_key);
        self.public_index.insert(arc_type, virtual_key);
        self.group_registrations.insert(
            element,
            GroupRegistration {
                element,
                virtual_key,
            },
        );
        self.group_by_type.insert(group_type, element);
        self.group_by_type.insert(arc_type, element);
        self.group_virtual_to_element.insert(virtual_key, element);
        Ok(())
    }
}

fn aggregate_group<T: Clone + Send + Sync + 'static>(
    container: &mut Container,
    element: TypeId,
    group_name: &'static str,
) -> Result<Constructed> {
    let members = container
        .group_members_for_element(element)
        .map(<[ProviderKey]>::to_vec)
        .unwrap_or_default();
    let mut items = Vec::with_capacity(members.len());
    for key in members {
        let value = container
            .take_group_member(key)
            .ok_or_else(|| Error::NotConstructed(group_name))?;
        items.push(take_packed_member::<T>(value)?);
    }
    Ok(pack(Group::from_vec(items)))
}
