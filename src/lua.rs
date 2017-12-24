use std::{ptr, str};
use std::ops::{Deref, DerefMut};
use std::iter::FromIterator;
use std::cell::RefCell;
use std::ffi::CString;
use std::any::TypeId;
use std::marker::PhantomData;
use std::collections::{HashMap, VecDeque};
use std::os::raw::{c_char, c_int, c_void};
use std::process;

use libc;

use ffi;
use error::*;
use util::*;
use types::{Callback, Integer, LightUserData, LuaRef, Number};
use string::String;
use table::Table;
use userdata::{AnyUserData, MetaMethod, UserData, UserDataMethods};

/// A dynamically typed Lua value.
#[derive(Debug, Clone)]
pub enum Value<'lua> {
    /// The Lua value `nil`.
    Nil,
    /// The Lua value `true` or `false`.
    Boolean(bool),
    /// A "light userdata" object, equivalent to a raw pointer.
    LightUserData(LightUserData),
    /// An integer number.
    ///
    /// Any Lua number convertible to a `Integer` will be represented as this variant.
    Integer(Integer),
    /// A floating point number.
    Number(Number),
    /// An interned string, managed by Lua.
    ///
    /// Unlike Rust strings, Lua strings may not be valid UTF-8.
    String(String<'lua>),
    /// Reference to a Lua table.
    Table(Table<'lua>),
    /// Reference to a Lua function (or closure).
    Function(Function<'lua>),
    /// Reference to a Lua thread (or coroutine).
    Thread(Thread<'lua>),
    /// Reference to a userdata object that holds a custom type which implements `UserData`.
    /// Special builtin userdata types will be represented as other `Value` variants.
    UserData(AnyUserData<'lua>),
    /// `Error` is a special builtin userdata type.  When received from Lua it is implicitly cloned.
    Error(Error),
}
pub use self::Value::Nil;

impl<'lua> Value<'lua> {
    pub(crate) fn type_name(&self) -> &'static str {
        match *self {
            Value::Nil => "nil",
            Value::Boolean(_) => "boolean",
            Value::LightUserData(_) => "light userdata",
            Value::Integer(_) => "integer",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Function(_) => "function",
            Value::Thread(_) => "thread",
            Value::UserData(_) | Value::Error(_) => "userdata",
        }
    }
}

/// Trait for types convertible to `Value`.
pub trait ToLua<'lua> {
    /// Performs the conversion.
    fn to_lua(self, lua: &'lua Lua) -> Result<Value<'lua>>;
}

/// Trait for types convertible from `Value`.
pub trait FromLua<'lua>: Sized {
    /// Performs the conversion.
    fn from_lua(lua_value: Value<'lua>, lua: &'lua Lua) -> Result<Self>;
}

/// Multiple Lua values used for both argument passing and also for multiple return values.
#[derive(Debug, Clone)]
pub struct MultiValue<'lua>(VecDeque<Value<'lua>>);

impl<'lua> MultiValue<'lua> {
    /// Creates an empty `MultiValue` containing no values.
    pub fn new() -> MultiValue<'lua> {
        MultiValue(VecDeque::new())
    }
}

impl<'lua> FromIterator<Value<'lua>> for MultiValue<'lua> {
    fn from_iter<I: IntoIterator<Item = Value<'lua>>>(iter: I) -> Self {
        MultiValue(VecDeque::from_iter(iter))
    }
}

impl<'lua> IntoIterator for MultiValue<'lua> {
    type Item = Value<'lua>;
    type IntoIter = <VecDeque<Value<'lua>> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'lua> Deref for MultiValue<'lua> {
    type Target = VecDeque<Value<'lua>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'lua> DerefMut for MultiValue<'lua> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Trait for types convertible to any number of Lua values.
///
/// This is a generalization of `ToLua`, allowing any number of resulting Lua values instead of just
/// one. Any type that implements `ToLua` will automatically implement this trait.
pub trait ToLuaMulti<'lua> {
    /// Performs the conversion.
    fn to_lua_multi(self, lua: &'lua Lua) -> Result<MultiValue<'lua>>;
}

/// Trait for types that can be created from an arbitrary number of Lua values.
///
/// This is a generalization of `FromLua`, allowing an arbitrary number of Lua values to participate
/// in the conversion. Any type that implements `FromLua` will automatically implement this trait.
pub trait FromLuaMulti<'lua>: Sized {
    /// Performs the conversion.
    ///
    /// In case `values` contains more values than needed to perform the conversion, the excess
    /// values should be ignored. This reflects the semantics of Lua when calling a function or
    /// assigning values. Similarly, if not enough values are given, conversions should assume that
    /// any missing values are nil.
    fn from_lua_multi(values: MultiValue<'lua>, lua: &'lua Lua) -> Result<Self>;
}

/// Handle to an internal Lua function.
#[derive(Clone, Debug)]
pub struct Function<'lua>(LuaRef<'lua>);

impl<'lua> Function<'lua> {
    /// Calls the function, passing `args` as function arguments.
    ///
    /// The function's return values are converted to the generic type `R`.
    ///
    /// # Examples
    ///
    /// Call Lua's built-in `tostring` function:
    ///
    /// ```
    /// # extern crate rlua;
    /// # use rlua::{Lua, Function, Result};
    /// # fn try_main() -> Result<()> {
    /// let lua = Lua::new();
    /// let globals = lua.globals();
    ///
    /// let tostring: Function = globals.get("tostring")?;
    ///
    /// assert_eq!(tostring.call::<_, String>(123)?, "123");
    ///
    /// # Ok(())
    /// # }
    /// # fn main() {
    /// #     try_main().unwrap();
    /// # }
    /// ```
    ///
    /// Call a function with multiple arguments:
    ///
    /// ```
    /// # extern crate rlua;
    /// # use rlua::{Lua, Function, Result};
    /// # fn try_main() -> Result<()> {
    /// let lua = Lua::new();
    ///
    /// let sum: Function = lua.eval(r#"
    ///     function(a, b)
    ///         return a + b
    ///     end
    /// "#, None)?;
    ///
    /// assert_eq!(sum.call::<_, u32>((3, 4))?, 3 + 4);
    ///
    /// # Ok(())
    /// # }
    /// # fn main() {
    /// #     try_main().unwrap();
    /// # }
    /// ```
    pub fn call<A: ToLuaMulti<'lua>, R: FromLuaMulti<'lua>>(&self, args: A) -> Result<R> {
        let lua = self.0.lua;
        unsafe {
            stack_err_guard(lua.state, 0, || {
                let args = args.to_lua_multi(lua)?;
                let nargs = args.len() as c_int;
                check_stack(lua.state, nargs + 3);

                let stack_start = ffi::lua_gettop(lua.state);
                lua.push_ref(lua.state, &self.0);
                for arg in args {
                    lua.push_value(lua.state, arg);
                }
                handle_error(
                    lua.state,
                    pcall_with_traceback(lua.state, nargs, ffi::LUA_MULTRET),
                )?;
                let nresults = ffi::lua_gettop(lua.state) - stack_start;
                let mut results = MultiValue::new();
                check_stack(lua.state, 1);
                for _ in 0..nresults {
                    results.push_front(lua.pop_value(lua.state));
                }
                R::from_lua_multi(results, lua)
            })
        }
    }

    /// Returns a function that, when called, calls `self`, passing `args` as the first set of
    /// arguments.
    ///
    /// If any arguments are passed to the returned function, they will be passed after `args`.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate rlua;
    /// # use rlua::{Lua, Function, Result};
    /// # fn try_main() -> Result<()> {
    /// let lua = Lua::new();
    ///
    /// let sum: Function = lua.eval(r#"
    ///     function(a, b)
    ///         return a + b
    ///     end
    /// "#, None)?;
    ///
    /// let bound_a = sum.bind(1)?;
    /// assert_eq!(bound_a.call::<_, u32>(2)?, 1 + 2);
    ///
    /// let bound_a_and_b = sum.bind(13)?.bind(57)?;
    /// assert_eq!(bound_a_and_b.call::<_, u32>(())?, 13 + 57);
    ///
    /// # Ok(())
    /// # }
    /// # fn main() {
    /// #     try_main().unwrap();
    /// # }
    /// ```
    pub fn bind<A: ToLuaMulti<'lua>>(&self, args: A) -> Result<Function<'lua>> {
        unsafe extern "C" fn bind_call_impl(state: *mut ffi::lua_State) -> c_int {
            let nargs = ffi::lua_gettop(state);
            let nbinds = ffi::lua_tointeger(state, lua_upvalueindex!(2)) as c_int;
            check_stack(state, nbinds + 2);

            ffi::lua_settop(state, nargs + nbinds + 1);
            ffi::lua_rotate(state, -(nargs + nbinds + 1), nbinds + 1);

            ffi::lua_pushvalue(state, lua_upvalueindex!(1));
            ffi::lua_replace(state, 1);

            for i in 0..nbinds {
                ffi::lua_pushvalue(state, lua_upvalueindex!(i + 3));
                ffi::lua_replace(state, i + 2);
            }

            ffi::lua_call(state, nargs + nbinds, ffi::LUA_MULTRET);
            ffi::lua_gettop(state)
        }

        let lua = self.0.lua;
        unsafe {
            stack_err_guard(lua.state, 0, || {
                let args = args.to_lua_multi(lua)?;
                let nargs = args.len() as c_int;

                check_stack(lua.state, nargs + 3);
                lua.push_ref(lua.state, &self.0);
                ffi::lua_pushinteger(lua.state, nargs as ffi::lua_Integer);
                for arg in args {
                    lua.push_value(lua.state, arg);
                }

                lua_pushcclosure!(lua.state, bind_call_impl, nargs + 2);

                Ok(Function(lua.pop_ref(lua.state)))
            })
        }
    }
}

/// Status of a Lua thread (or coroutine).
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ThreadStatus {
    /// The thread was just created, or is suspended because it has called `coroutine.yield`.
    ///
    /// If a thread is in this state, it can be resumed by calling [`Thread::resume`].
    ///
    /// [`Thread::resume`]: struct.Thread.html#method.resume
    Resumable,
    /// Either the thread has finished executing, or the thread is currently running.
    Unresumable,
    /// The thread has raised a Lua error during execution.
    Error,
}

/// Handle to an internal Lua thread (or coroutine).
#[derive(Clone, Debug)]
pub struct Thread<'lua>(LuaRef<'lua>);

impl<'lua> Thread<'lua> {
    /// Resumes execution of this thread.
    ///
    /// Equivalent to `coroutine.resume`.
    ///
    /// Passes `args` as arguments to the thread. If the coroutine has called `coroutine.yield`, it
    /// will return these arguments. Otherwise, the coroutine wasn't yet started, so the arguments
    /// are passed to its main function.
    ///
    /// If the thread is no longer in `Active` state (meaning it has finished execution or
    /// encountered an error), this will return `Err(CoroutineInactive)`, otherwise will return `Ok`
    /// as follows:
    ///
    /// If the thread calls `coroutine.yield`, returns the values passed to `yield`. If the thread
    /// `return`s values from its main function, returns those.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate rlua;
    /// # use rlua::{Lua, Thread, Error, Result};
    /// # fn try_main() -> Result<()> {
    /// let lua = Lua::new();
    /// let thread: Thread = lua.eval(r#"
    ///     coroutine.create(function(arg)
    ///         assert(arg == 42)
    ///         local yieldarg = coroutine.yield(123)
    ///         assert(yieldarg == 43)
    ///         return 987
    ///     end)
    /// "#, None).unwrap();
    ///
    /// assert_eq!(thread.resume::<_, u32>(42).unwrap(), 123);
    /// assert_eq!(thread.resume::<_, u32>(43).unwrap(), 987);
    ///
    /// // The coroutine has now returned, so `resume` will fail
    /// match thread.resume::<_, u32>(()) {
    ///     Err(Error::CoroutineInactive) => {},
    ///     unexpected => panic!("unexpected result {:?}", unexpected),
    /// }
    /// # Ok(())
    /// # }
    /// # fn main() {
    /// #     try_main().unwrap();
    /// # }
    /// ```
    pub fn resume<A, R>(&self, args: A) -> Result<R>
    where
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        let lua = self.0.lua;
        unsafe {
            stack_err_guard(lua.state, 0, || {
                check_stack(lua.state, 1);

                lua.push_ref(lua.state, &self.0);
                let thread_state = ffi::lua_tothread(lua.state, -1);

                let status = ffi::lua_status(thread_state);
                if status != lua_yield!() && ffi::lua_gettop(thread_state) == 0 {
                    return Err(Error::CoroutineInactive);
                }

                lua_pop!(lua.state, 1);

                let args = args.to_lua_multi(lua)?;
                let nargs = args.len() as c_int;
                check_stack(thread_state, nargs);

                for arg in args {
                    lua.push_value(thread_state, arg);
                }

                handle_error(
                    thread_state,
                    resume_with_traceback(thread_state, lua.state, nargs),
                )?;

                let nresults = ffi::lua_gettop(thread_state);
                let mut results = MultiValue::new();
                check_stack(thread_state, 1);
                for _ in 0..nresults {
                    results.push_front(lua.pop_value(thread_state));
                }
                R::from_lua_multi(results, lua)
            })
        }
    }

    /// Gets the status of the thread.
    pub fn status(&self) -> ThreadStatus {
        let lua = self.0.lua;
        unsafe {
            stack_guard(lua.state, 0, || {
                check_stack(lua.state, 1);

                lua.push_ref(lua.state, &self.0);
                let thread_state = ffi::lua_tothread(lua.state, -1);
                lua_pop!(lua.state, 1);

                let status = ffi::lua_status(thread_state);
                if status != lua_ok!() && status != lua_yield!() {
                    ThreadStatus::Error
                } else if status == lua_yield!() || ffi::lua_gettop(thread_state) > 0 {
                    ThreadStatus::Resumable
                } else {
                    ThreadStatus::Unresumable
                }
            })
        }
    }
}

/// Top level Lua struct which holds the Lua state itself.
pub struct Lua {
    pub(crate) state: *mut ffi::lua_State,
    main_state: *mut ffi::lua_State,
    ephemeral: bool,
}

impl Drop for Lua {
    fn drop(&mut self) {
        unsafe {
            if !self.ephemeral {
                ffi::lua_close(self.state);
            }
        }
    }
}

impl Lua {
    /// Creates a new Lua state.
    ///
    /// Also loads the standard library.
    pub fn new() -> Lua {
        unsafe extern "C" fn allocator(
            _: *mut c_void,
            ptr: *mut c_void,
            _: usize,
            nsize: usize,
        ) -> *mut c_void {
            if nsize == 0 {
                libc::free(ptr as *mut libc::c_void);
                ptr::null_mut()
            } else {
                let p = libc::realloc(ptr as *mut libc::c_void, nsize);
                if p.is_null() {
                    // We must abort on OOM, because otherwise this will result in an unsafe
                    // longjmp.
                    eprintln!("Out of memory in Lua allocation, aborting!");
                    process::abort()
                } else {
                    p as *mut c_void
                }
            }
        }

        unsafe {
            let state = lua_newstate!(allocator, ptr::null_mut());

            stack_guard(state, 0, || {
                // Do not open the debug library, currently it can be used to cause unsafety.
                ffi::luaL_requiref(state, cstr!("_G"), ffi::luaopen_base, 1);
                ffi::luaL_requiref(state, cstr!("coroutine"), ffi::luaopen_coroutine, 1);
                ffi::luaL_requiref(state, cstr!("table"), ffi::luaopen_table, 1);
                ffi::luaL_requiref(state, cstr!("io"), ffi::luaopen_io, 1);
                ffi::luaL_requiref(state, cstr!("os"), ffi::luaopen_os, 1);
                ffi::luaL_requiref(state, cstr!("string"), ffi::luaopen_string, 1);
                ffi::luaL_requiref(state, cstr!("utf8"), ffi::luaopen_utf8, 1);
                ffi::luaL_requiref(state, cstr!("math"), ffi::luaopen_math, 1);
                ffi::luaL_requiref(state, cstr!("package"), ffi::luaopen_package, 1);
                lua_pop!(state, 9);

                // Create the userdata registry table

                ffi::lua_pushlightuserdata(
                    state,
                    &LUA_USERDATA_REGISTRY_KEY as *const u8 as *mut c_void,
                );

                push_userdata::<HashMap<TypeId, c_int>>(state, HashMap::new());

                lua_newtable!(state);

                push_string(state, "__gc");
                lua_pushcfunction!(state, userdata_destructor::<HashMap<TypeId, c_int>>);
                ffi::lua_rawset(state, -3);

                ffi::lua_setmetatable(state, -2);

                ffi::lua_rawset(state, ffi::LUA_REGISTRYINDEX);

                // Create the function metatable

                ffi::lua_pushlightuserdata(
                    state,
                    &FUNCTION_METATABLE_REGISTRY_KEY as *const u8 as *mut c_void,
                );

                lua_newtable!(state);

                push_string(state, "__gc");
                lua_pushcfunction!(state, userdata_destructor::<RefCell<Callback>>);
                ffi::lua_rawset(state, -3);

                push_string(state, "__metatable");
                ffi::lua_pushboolean(state, 0);
                ffi::lua_rawset(state, -3);

                ffi::lua_rawset(state, ffi::LUA_REGISTRYINDEX);

                // Override pcall, xpcall, and setmetatable with versions that cannot be used to
                // cause unsafety.

                ffi::lua_rawgeti(state, ffi::LUA_REGISTRYINDEX, lua_ridx_globals!());

                push_string(state, "pcall");
                lua_pushcfunction!(state, safe_pcall);
                ffi::lua_rawset(state, -3);

                push_string(state, "xpcall");
                lua_pushcfunction!(state, safe_xpcall);
                ffi::lua_rawset(state, -3);

                push_string(state, "setmetatable");
                lua_pushcfunction!(state, safe_setmetatable);
                ffi::lua_rawset(state, -3);

                lua_pop!(state, 1);
            });

            Lua {
                state,
                main_state: state,
                ephemeral: false,
            }
        }
    }

    /// Loads the Lua debug library.
    ///
    /// The debug library is very unsound, loading it and using it breaks all
    /// the guarantees of rlua.
    pub unsafe fn load_debug(&self) {
        check_stack(self.state, 1);
        ffi::luaL_requiref(self.state, cstr!("debug"), ffi::luaopen_debug, 1);
        lua_pop!(self.state, 1);
    }

    /// Loads a chunk of Lua code and returns it as a function.
    ///
    /// The source can be named by setting the `name` parameter. This is generally recommended as it
    /// results in better error traces.
    ///
    /// Equivalent to Lua's `load` function.
    pub fn load(&self, source: &str, name: Option<&str>) -> Result<Function> {
        unsafe {
            stack_err_guard(self.state, 0, || {
                check_stack(self.state, 1);

                handle_error(
                    self.state,
                    if let Some(name) = name {
                        let name = CString::new(name.to_owned()).map_err(|e| {
                            Error::ToLuaConversionError {
                                from: "&str",
                                to: "string",
                                message: Some(e.to_string()),
                            }
                        })?;
                        ffi::luaL_loadbuffer(
                            self.state,
                            source.as_ptr() as *const c_char,
                            source.len(),
                            name.as_ptr(),
                        )
                    } else {
                        ffi::luaL_loadbuffer(
                            self.state,
                            source.as_ptr() as *const c_char,
                            source.len(),
                            ptr::null(),
                        )
                    },
                )?;

                Ok(Function(self.pop_ref(self.state)))
            })
        }
    }

    /// Execute a chunk of Lua code.
    ///
    /// This is equivalent to simply loading the source with `load` and then calling the resulting
    /// function with no arguments.
    ///
    /// Returns the values returned by the chunk.
    pub fn exec<'lua, R: FromLuaMulti<'lua>>(
        &'lua self,
        source: &str,
        name: Option<&str>,
    ) -> Result<R> {
        self.load(source, name)?.call(())
    }

    /// Evaluate the given expression or chunk inside this Lua state.
    ///
    /// If `source` is an expression, returns the value it evaluates to. Otherwise, returns the
    /// values returned by the chunk (if any).
    pub fn eval<'lua, R: FromLuaMulti<'lua>>(
        &'lua self,
        source: &str,
        name: Option<&str>,
    ) -> Result<R> {
        // First, try interpreting the lua as an expression by adding
        // "return", then as a statement.  This is the same thing the
        // actual lua repl does.
        self.load(&format!("return {}", source), name)
            .or_else(|_| self.load(source, name))?
            .call(())
    }

    /// Pass a `&str` slice to Lua, creating and returning an interned Lua string.
    pub fn create_string(&self, s: &str) -> String {
        unsafe {
            stack_guard(self.state, 0, || {
                check_stack(self.state, 2);
                ffi::lua_pushlstring(self.state, s.as_ptr() as *const c_char, s.len());
                String(self.pop_ref(self.state))
            })
        }
    }

    /// Creates and returns a new table.
    pub fn create_table(&self) -> Table {
        unsafe {
            stack_guard(self.state, 0, || {
                check_stack(self.state, 2);
                lua_newtable!(self.state);
                Table(self.pop_ref(self.state))
            })
        }
    }

    /// Creates a table and fills it with values from an iterator.
    pub fn create_table_from<'lua, K, V, I>(&'lua self, cont: I) -> Result<Table<'lua>>
    where
        K: ToLua<'lua>,
        V: ToLua<'lua>,
        I: IntoIterator<Item = (K, V)>,
    {
        unsafe {
            stack_err_guard(self.state, 0, || {
                check_stack(self.state, 3);
                lua_newtable!(self.state);

                for (k, v) in cont {
                    self.push_value(self.state, k.to_lua(self)?);
                    self.push_value(self.state, v.to_lua(self)?);
                    ffi::lua_rawset(self.state, -3);
                }
                Ok(Table(self.pop_ref(self.state)))
            })
        }
    }

    /// Creates a table from an iterator of values, using `1..` as the keys.
    pub fn create_sequence_from<'lua, T, I>(&'lua self, cont: I) -> Result<Table<'lua>>
    where
        T: ToLua<'lua>,
        I: IntoIterator<Item = T>,
    {
        self.create_table_from(cont.into_iter().enumerate().map(|(k, v)| (k + 1, v)))
    }

    /// Wraps a Rust function or closure, creating a callable Lua function handle to it.
    ///
    /// # Examples
    ///
    /// Create a function which prints its argument:
    ///
    /// ```
    /// # extern crate rlua;
    /// # use rlua::{Lua, Result};
    /// # fn try_main() -> Result<()> {
    /// let lua = Lua::new();
    ///
    /// let greet = lua.create_function(|_, name: String| {
    ///     println!("Hello, {}!", name);
    ///     Ok(())
    /// });
    /// # let _ = greet;    // used
    /// # Ok(())
    /// # }
    /// # fn main() {
    /// #     try_main().unwrap();
    /// # }
    /// ```
    ///
    /// Use tuples to accept multiple arguments:
    ///
    /// ```
    /// # extern crate rlua;
    /// # use rlua::{Lua, Result};
    /// # fn try_main() -> Result<()> {
    /// let lua = Lua::new();
    ///
    /// let print_person = lua.create_function(|_, (name, age): (String, u8)| {
    ///     println!("{} is {} years old!", name, age);
    ///     Ok(())
    /// });
    /// # let _ = print_person;    // used
    /// # Ok(())
    /// # }
    /// # fn main() {
    /// #     try_main().unwrap();
    /// # }
    /// ```
    pub fn create_function<'lua, A, R, F>(&'lua self, mut func: F) -> Function<'lua>
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + FnMut(&'lua Lua, A) -> Result<R>,
    {
        self.create_callback_function(Box::new(move |lua, args| {
            func(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
        }))
    }

    /// Wraps a Lua function into a new thread (or coroutine).
    ///
    /// Equivalent to `coroutine.create`.
    pub fn create_thread<'lua>(&'lua self, func: Function<'lua>) -> Thread<'lua> {
        unsafe {
            stack_guard(self.state, 0, move || {
                check_stack(self.state, 2);

                let thread_state = ffi::lua_newthread(self.state);
                self.push_ref(thread_state, &func.0);

                Thread(self.pop_ref(self.state))
            })
        }
    }

    /// Create a Lua userdata object from a custom userdata type.
    pub fn create_userdata<T>(&self, data: T) -> AnyUserData
    where
        T: UserData,
    {
        unsafe {
            stack_guard(self.state, 0, move || {
                check_stack(self.state, 3);

                push_userdata::<RefCell<T>>(self.state, RefCell::new(data));

                ffi::lua_rawgeti(
                    self.state,
                    ffi::LUA_REGISTRYINDEX,
                    self.userdata_metatable::<T>() as ffi::lua_Integer,
                );

                ffi::lua_setmetatable(self.state, -2);

                AnyUserData(self.pop_ref(self.state))
            })
        }
    }

    /// Returns a handle to the global environment.
    pub fn globals(&self) -> Table {
        unsafe {
            stack_guard(self.state, 0, move || {
                check_stack(self.state, 2);
                ffi::lua_rawgeti(self.state, ffi::LUA_REGISTRYINDEX, lua_ridx_globals!());
                Table(self.pop_ref(self.state))
            })
        }
    }

    /// Coerces a Lua value to a string.
    ///
    /// The value must be a string (in which case this is a no-op) or a number.
    pub fn coerce_string<'lua>(&'lua self, v: Value<'lua>) -> Result<String<'lua>> {
        match v {
            Value::String(s) => Ok(s),
            v => unsafe {
                stack_guard(self.state, 0, || {
                    check_stack(self.state, 2);
                    let ty = v.type_name();
                    self.push_value(self.state, v);
                    if lua_tostring!(self.state, -1).is_null() {
                        lua_pop!(self.state, 1);
                        Err(Error::FromLuaConversionError {
                            from: ty,
                            to: "String",
                            message: Some("expected string or number".to_string()),
                        })
                    } else {
                        Ok(String(self.pop_ref(self.state)))
                    }
                })
            },
        }
    }

    /// Coerces a Lua value to an integer.
    ///
    /// The value must be an integer, or a floating point number or a string that can be converted
    /// to an integer. Refer to the Lua manual for details.
    pub fn coerce_integer(&self, v: Value) -> Result<Integer> {
        match v {
            Value::Integer(i) => Ok(i),
            v => unsafe {
                stack_guard(self.state, 0, || {
                    check_stack(self.state, 1);
                    let ty = v.type_name();
                    self.push_value(self.state, v);
                    let mut isint = 0;
                    let i = ffi::lua_tointegerx(self.state, -1, &mut isint);
                    lua_pop!(self.state, 1);
                    if isint == 0 {
                        Err(Error::FromLuaConversionError {
                            from: ty,
                            to: "integer",
                            message: None,
                        })
                    } else {
                        Ok(i)
                    }
                })
            },
        }
    }

    /// Coerce a Lua value to a number.
    ///
    /// The value must be a number or a string that can be converted to a number. Refer to the Lua
    /// manual for details.
    pub fn coerce_number(&self, v: Value) -> Result<Number> {
        match v {
            Value::Number(n) => Ok(n),
            v => unsafe {
                stack_guard(self.state, 0, || {
                    check_stack(self.state, 1);
                    let ty = v.type_name();
                    self.push_value(self.state, v);
                    let mut isnum = 0;
                    let n = ffi::lua_tonumberx(self.state, -1, &mut isnum);
                    lua_pop!(self.state, 1);
                    if isnum == 0 {
                        Err(Error::FromLuaConversionError {
                            from: ty,
                            to: "number",
                            message: Some("number or string coercible to number".to_string()),
                        })
                    } else {
                        Ok(n)
                    }
                })
            },
        }
    }

    /// Converts a value that implements `ToLua` into a `Value` instance.
    pub fn pack<'lua, T: ToLua<'lua>>(&'lua self, t: T) -> Result<Value<'lua>> {
        t.to_lua(self)
    }

    /// Converts a `Value` instance into a value that implements `FromLua`.
    pub fn unpack<'lua, T: FromLua<'lua>>(&'lua self, value: Value<'lua>) -> Result<T> {
        T::from_lua(value, self)
    }

    /// Converts a value that implements `ToLuaMulti` into a `MultiValue` instance.
    pub fn pack_multi<'lua, T: ToLuaMulti<'lua>>(&'lua self, t: T) -> Result<MultiValue<'lua>> {
        t.to_lua_multi(self)
    }

    /// Converts a `MultiValue` instance into a value that implements `FromLuaMulti`.
    pub fn unpack_multi<'lua, T: FromLuaMulti<'lua>>(
        &'lua self,
        value: MultiValue<'lua>,
    ) -> Result<T> {
        T::from_lua_multi(value, self)
    }

    fn create_callback_function<'lua>(&'lua self, func: Callback<'lua>) -> Function<'lua> {
        unsafe extern "C" fn callback_call_impl(state: *mut ffi::lua_State) -> c_int {
            callback_error(state, || {
                let lua = Lua {
                    state: state,
                    main_state: main_state(state),
                    ephemeral: true,
                };

                let func = get_userdata::<RefCell<Callback>>(state, lua_upvalueindex!(1));
                let mut func = if let Ok(func) = (*func).try_borrow_mut() {
                    func
                } else {
                    lua_panic!(
                        state,
                        "recursive callback function call would mutably borrow function twice"
                    );
                };

                let nargs = ffi::lua_gettop(state);
                let mut args = MultiValue::new();
                check_stack(state, 1);
                for _ in 0..nargs {
                    args.push_front(lua.pop_value(state));
                }

                let results = func.deref_mut()(&lua, args)?;
                let nresults = results.len() as c_int;

                check_stack(state, nresults);

                for r in results {
                    lua.push_value(state, r);
                }

                Ok(nresults)
            })
        }

        unsafe {
            stack_guard(self.state, 0, move || {
                check_stack(self.state, 2);

                push_userdata::<RefCell<Callback>>(self.state, RefCell::new(func));

                ffi::lua_pushlightuserdata(
                    self.state,
                    &FUNCTION_METATABLE_REGISTRY_KEY as *const u8 as *mut c_void,
                );
                ffi::lua_gettable(self.state, ffi::LUA_REGISTRYINDEX);
                ffi::lua_setmetatable(self.state, -2);

                lua_pushcclosure!(self.state, callback_call_impl, 1);

                Function(self.pop_ref(self.state))
            })
        }
    }

    // Used 1 stack space, does not call checkstack
    pub(crate) unsafe fn push_value(&self, state: *mut ffi::lua_State, value: Value) {
        match value {
            Value::Nil => {
                ffi::lua_pushnil(state);
            }

            Value::Boolean(b) => {
                ffi::lua_pushboolean(state, if b { 1 } else { 0 });
            }

            Value::LightUserData(ud) => {
                ffi::lua_pushlightuserdata(state, ud.0);
            }

            Value::Integer(i) => {
                ffi::lua_pushinteger(state, i);
            }

            Value::Number(n) => {
                ffi::lua_pushnumber(state, n);
            }

            Value::String(s) => {
                self.push_ref(state, &s.0);
            }

            Value::Table(t) => {
                self.push_ref(state, &t.0);
            }

            Value::Function(f) => {
                self.push_ref(state, &f.0);
            }

            Value::Thread(t) => {
                self.push_ref(state, &t.0);
            }

            Value::UserData(ud) => {
                self.push_ref(state, &ud.0);
            }

            Value::Error(e) => {
                push_wrapped_error(state, e);
            }
        }
    }

    // Used 1 stack space, does not call checkstack
    pub(crate) unsafe fn pop_value(&self, state: *mut ffi::lua_State) -> Value {
        match ffi::lua_type(state, -1) {
            lua_tnil!() => {
                lua_pop!(state, 1);
                Nil
            }

            lua_tboolean!() => {
                let b = Value::Boolean(ffi::lua_toboolean(state, -1) != 0);
                lua_pop!(state, 1);
                b
            }

            lua_tlightuserdata!() => {
                let ud = Value::LightUserData(LightUserData(ffi::lua_touserdata(state, -1)));
                lua_pop!(state, 1);
                ud
            }

            lua_tnumber!() => if ffi::lua_isinteger(state, -1) != 0 {
                let i = Value::Integer(ffi::lua_tointeger(state, -1));
                lua_pop!(state, 1);
                i
            } else {
                let n = Value::Number(ffi::lua_tonumber(state, -1));
                lua_pop!(state, 1);
                n
            },

            lua_tstring!() => Value::String(String(self.pop_ref(state))),

            lua_ttable!() => Value::Table(Table(self.pop_ref(state))),

            lua_tfunction!() => Value::Function(Function(self.pop_ref(state))),

            lua_tuserdata!() => {
                // It should not be possible to interact with userdata types
                // other than custom UserData types OR a WrappedError.
                // WrappedPanic should never be able to be caught in lua, so it
                // should never be here.
                if let Some(err) = pop_wrapped_error(state) {
                    Value::Error(err)
                } else {
                    Value::UserData(AnyUserData(self.pop_ref(state)))
                }
            }

            lua_tthread!() => Value::Thread(Thread(self.pop_ref(state))),

            _ => unreachable!("internal error: LUA_TNONE in pop_value"),
        }
    }

    // Used 1 stack space, does not call checkstack
    pub(crate) unsafe fn push_ref(&self, state: *mut ffi::lua_State, lref: &LuaRef) {
        assert_eq!(
            lref.lua.main_state,
            self.main_state,
            "Lua instance passed Value created from a different Lua"
        );

        ffi::lua_rawgeti(
            state,
            ffi::LUA_REGISTRYINDEX,
            lref.registry_id as ffi::lua_Integer,
        );
    }

    // Pops the topmost element of the stack and stores a reference to it in the
    // registry.
    //
    // This pins the object, preventing garbage collection until the returned
    // `LuaRef` is dropped.
    //
    // pop_ref uses 1 extra stack space and does not call checkstack
    pub(crate) unsafe fn pop_ref(&self, state: *mut ffi::lua_State) -> LuaRef {
        let registry_id = ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX);
        LuaRef {
            lua: self,
            registry_id: registry_id,
        }
    }

    pub(crate) unsafe fn userdata_metatable<T: UserData>(&self) -> c_int {
        // Used if both an __index metamethod is set and regular methods, checks methods table
        // first, then __index metamethod.
        unsafe extern "C" fn meta_index_impl(state: *mut ffi::lua_State) -> c_int {
            check_stack(state, 2);

            ffi::lua_pushvalue(state, -1);
            ffi::lua_gettable(state, lua_upvalueindex!(1));
            if lua_isnil!(state, -1) {
                ffi::lua_insert(state, -3);
                lua_pop!(state, 2);
                1
            } else {
                lua_pop!(state, 1);
                ffi::lua_pushvalue(state, lua_upvalueindex!(2));
                ffi::lua_insert(state, -3);
                ffi::lua_call(state, 2, 1);
                1
            }
        }

        stack_guard(self.state, 0, move || {
            check_stack(self.state, 5);

            ffi::lua_pushlightuserdata(
                self.state,
                &LUA_USERDATA_REGISTRY_KEY as *const u8 as *mut c_void,
            );
            ffi::lua_gettable(self.state, ffi::LUA_REGISTRYINDEX);
            let registered_userdata = get_userdata::<HashMap<TypeId, c_int>>(self.state, -1);
            lua_pop!(self.state, 1);

            if let Some(table_id) = (*registered_userdata).get(&TypeId::of::<T>()) {
                return *table_id;
            }

            let mut methods = UserDataMethods {
                methods: HashMap::new(),
                meta_methods: HashMap::new(),
                _type: PhantomData,
            };
            T::add_methods(&mut methods);

            lua_newtable!(self.state);

            let has_methods = !methods.methods.is_empty();

            if has_methods {
                push_string(self.state, "__index");
                lua_newtable!(self.state);

                for (k, m) in methods.methods {
                    push_string(self.state, &k);
                    self.push_value(
                        self.state,
                        Value::Function(self.create_callback_function(m)),
                    );
                    ffi::lua_rawset(self.state, -3);
                }

                ffi::lua_rawset(self.state, -3);
            }

            for (k, m) in methods.meta_methods {
                if k == MetaMethod::Index && has_methods {
                    push_string(self.state, "__index");
                    ffi::lua_pushvalue(self.state, -1);
                    ffi::lua_gettable(self.state, -3);
                    self.push_value(
                        self.state,
                        Value::Function(self.create_callback_function(m)),
                    );
                    lua_pushcclosure!(self.state, meta_index_impl, 2);
                    ffi::lua_rawset(self.state, -3);
                } else {
                    let name = match k {
                        MetaMethod::Add => "__add",
                        MetaMethod::Sub => "__sub",
                        MetaMethod::Mul => "__mul",
                        MetaMethod::Div => "__div",
                        MetaMethod::Mod => "__mod",
                        MetaMethod::Pow => "__pow",
                        MetaMethod::Unm => "__unm",
                        MetaMethod::IDiv => "__idiv",
                        MetaMethod::BAnd => "__band",
                        MetaMethod::BOr => "__bor",
                        MetaMethod::BXor => "__bxor",
                        MetaMethod::BNot => "__bnot",
                        MetaMethod::Shl => "__shl",
                        MetaMethod::Shr => "__shr",
                        MetaMethod::Concat => "__concat",
                        MetaMethod::Len => "__len",
                        MetaMethod::Eq => "__eq",
                        MetaMethod::Lt => "__lt",
                        MetaMethod::Le => "__le",
                        MetaMethod::Index => "__index",
                        MetaMethod::NewIndex => "__newindex",
                        MetaMethod::Call => "__call",
                        MetaMethod::ToString => "__tostring",
                    };
                    push_string(self.state, name);
                    self.push_value(
                        self.state,
                        Value::Function(self.create_callback_function(m)),
                    );
                    ffi::lua_rawset(self.state, -3);
                }
            }

            push_string(self.state, "__gc");
            lua_pushcfunction!(self.state, userdata_destructor::<RefCell<T>>);
            ffi::lua_rawset(self.state, -3);

            push_string(self.state, "__metatable");
            ffi::lua_pushboolean(self.state, 0);
            ffi::lua_rawset(self.state, -3);

            let id = ffi::luaL_ref(self.state, ffi::LUA_REGISTRYINDEX);
            (*registered_userdata).insert(TypeId::of::<T>(), id);
            id
        })
    }
}

static LUA_USERDATA_REGISTRY_KEY: u8 = 0;
static FUNCTION_METATABLE_REGISTRY_KEY: u8 = 0;
