//! Registering implemented methods and reads against the published catalog.

use std::sync::Arc;

use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_protocol::schema::method::{Effect, MethodSpec};
use ovis_terminal_protocol::schema::methods::all_methods;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::super::read::Reader;
use super::super::registry::Registered;
use super::super::{DispatchError, Method, Read};
use super::Dispatcher;

impl<D, P> Dispatcher<D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// Registers an implemented module before sharing the dispatcher.
    ///
    /// # Errors
    ///
    /// Rejects unknown, duplicate, hello and built-in operation-control methods.
    pub fn register<M: Method<D, P>>(&mut self, method: M) -> Result<(), DispatchError> {
        let spec = self.declared::<M::Params>(M::NAME)?;
        self.routes.insert(
            M::NAME,
            Box::new(Registered {
                method: Arc::new(method),
                effect: spec.effect,
                targeting: spec.targeting,
            }),
        );
        Ok(())
    }

    /// Registers a query answered from a committed snapshot, outside the operation ledger.
    ///
    /// # Errors
    ///
    /// Rejects unknown, duplicate, non-query, hello and built-in operation-control methods.
    pub fn register_read<R: Read>(&mut self, reader: R) -> Result<(), DispatchError> {
        if !matches!(self.declared::<R::Params>(R::NAME)?.effect, Effect::Query) {
            return Err(DispatchError::Registration);
        }
        self.reads
            .insert(R::NAME, Box::new(Reader(Arc::new(reader))));
        Ok(())
    }

    /// The catalog entry of `name`, if it is unregistered and declares exactly `T` as params.
    fn declared<T: JsonSchema>(&self, name: &str) -> Result<&'static MethodSpec, DispatchError> {
        let spec = all_methods()
            .find(|spec| spec.name == name)
            .ok_or(DispatchError::Registration)?;
        if ["hello", "op.get", "op.cancel"].contains(&name)
            || self.routes.contains_key(name)
            || self.reads.contains_key(name)
        {
            return Err(DispatchError::Registration);
        }
        let mut declared = schemars::SchemaGenerator::default();
        let mut registered = schemars::SchemaGenerator::default();
        if (spec.params)(&mut declared) != registered.subschema_for::<T>()
            || declared.definitions() != registered.definitions()
        {
            return Err(DispatchError::Registration);
        }
        Ok(spec)
    }
}
