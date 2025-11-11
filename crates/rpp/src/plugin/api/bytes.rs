use mlua::UserData;

#[derive(Debug)]
pub struct Bytes(Vec<u8>);

impl UserData for Bytes {
    fn add_fields<F: mlua::UserDataFields<Self>>(_: &mut F) {}

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("len", |_, this, _: ()| Ok(this.0.len()));

        methods.add_method("get", |_, this, idx: usize| {
            this.0
                .get(idx)
                .copied()
                .ok_or_else(|| mlua::Error::external(format!("Index {} out of bounds", idx)))
        });

        methods.add_method_mut("push", |_, this, val: u8| {
            this.0.push(val);
            Ok(())
        });

        methods.add_method_mut("set", |_, this, (idx, val): (usize, u8)| {
            if idx >= this.0.len() {
                return Err(mlua::Error::external(format!(
                    "Index {} out of bounds",
                    idx
                )));
            }
            this.0[idx] = val;
            Ok(())
        });

        methods.add_method_mut("remove", |_, this, idx: usize| {
            if idx >= this.0.len() {
                return Err(mlua::Error::external(format!(
                    "Index {} out of bounds",
                    idx
                )));
            }
            Ok(this.0.remove(idx))
        });

        methods.add_method_mut("clear", |_, this, _: ()| {
            this.0.clear();
            Ok(())
        });

        methods.add_method_mut("resize", |_, this, (len, default): (usize, u8)| {
            this.0.resize(len, default);
            Ok(())
        });

        methods.add_function("new", |_, capacity: Option<usize>| {
            Ok(Bytes(Vec::with_capacity(capacity.unwrap_or(0))))
        });

        methods.add_meta_method(mlua::MetaMethod::Len, |_, this, ()| Ok(this.0.len()));

        methods.add_meta_method(mlua::MetaMethod::Index, |_, this, idx: usize| {
            this.0
                .get(idx)
                .copied()
                .ok_or_else(|| mlua::Error::external(format!("Index {} out of bounds", idx)))
        });
    }
}
