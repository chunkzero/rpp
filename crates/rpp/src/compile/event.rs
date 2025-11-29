use std::ops::Deref;

use crate::compile::context::BuildContext;

pub enum BuildEvent<'a> {
    Begin(&'a dyn BuildContext),
    ProcessFile(&'a dyn BuildContext),
    End(&'a dyn BuildContext),
}

pub trait EventHandler {
    fn id(&self) -> String;

    fn handle_event(&self, event: BuildEvent) -> crate::Result<()>;
}

impl<T, P> EventHandler for P
where
    P: Deref<Target = T>,
    T: EventHandler,
{
    fn id(&self) -> String {
        T::id(self)
    }

    fn handle_event(&self, event: BuildEvent) -> crate::Result<()> {
        T::handle_event(self, event)
    }
}
