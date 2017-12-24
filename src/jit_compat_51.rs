#![allow(unused_macros)]

macro_rules! lua_copy {
    ($lua_State:expr, $fromidx:expr, $toidx:expr) => ({
        TValue *fr, *to;
        ffi::lua_lock($lua_State);
        let fr = ffi::index2addr($lua_State, $fromidx);
        let to = ffi::index2addr($lua_State, $toidx);
        ffi::api_checkvalidindex($lua_State, to);
        ffi::setobj($lua_State, to, fr);
        if (isupvalue(toidx))  {
            /* function upvalue? */
            ffi::luaC_barrier($lua_State, clCvalue(L->ci->func), fr);
        }

        /* LUA_REGISTRYINDEX does not need gc barrier
         (collector revisits it before finishing collection) */
        ffi::lua_unlock($lua_State);
    })
}

macro_rules! lua_lock {($lua_State:expr) => (::std::ptr::null())}
macro_rules! lua_unlock {($lua_State:expr) => (::std::ptr::null())}
macro_rules! clCvalue {($o:expr) => (ffi::check_exp(ttisCclosure($o), gco2ccl(val_($o).gc))) }

macro_rules! lua_tostring {
    ($lua_State:expr, $i:expr) => (ffi::lua_tolstring($lua_State, $i, ::std::ptr::null_mut()))
}

macro_rules! ispseudo {
    ($i:expr) => ($i <= ffi::LUA_REGISTRYINDEX)
}

macro_rules! lua_upvalueindex {
    ($i:expr) => ((ffi::LUA_GLOBALSINDEX - $i))
}
macro_rules! lua_newstate {
    ($allocator:expr, $ptr:expr) => (ffi::lua_newstate(Some($allocator), $ptr))
}
macro_rules! lua_absindex {
    ($lua_State:expr, $idx:expr) => ({
        if $idx > 0 || ispseudo!($idx) {
            $idx
        } else {
            ffi::lua_gettop($lua_State) + $idx
        }
    })
}

macro_rules! lua_pushcfunction {
    ($lua_State:expr, $function:expr) => ({
        lua_pushcclosure!($lua_State, $function, 0)
    })
}

macro_rules! lua_pop {
    ($lua_State:expr, $i:expr) => (ffi::lua_settop($lua_State, -($i)-1))
}

macro_rules! lua_newtable {
    ($lua_State:expr) => (ffi::lua_createtable($lua_State, 0, 0))
}
macro_rules! lua_ridx_globals {
    () => (/* #define LUA_RIDX_GLOBALS  2 */ 2)
}


macro_rules! lua_pushcclosure {
    ($lua_State:expr, $fn:expr, $i:expr) => (ffi::lua_pushcclosure($lua_State, Some($fn), $i))
}

// type codes
macro_rules! lua_tnone              {() => (/*#define LUA_TNONE     (-1)*/ -1)}
macro_rules! lua_tnil               {() => (/*#define LUA_TNIL       0*/ 0)}
macro_rules! lua_tboolean           {() => (/*#define LUA_TBOOLEAN     1*/ 1)}
macro_rules! lua_tlightuserdata     {() => (/*#define LUA_TLIGHTUSERDATA     2*/ 2)}
macro_rules! lua_tnumber            {() => (/*#define LUA_TNUMBER     3*/ 3)}
macro_rules! lua_tstring            {() => (/*#define LUA_TSTRING     4*/ 4)}
macro_rules! lua_ttable             {() => (/*#define LUA_TTABLE     5*/ 5)}
macro_rules! lua_tfunction          {() => (/*#define LUA_TFUNCTION       6*/ 6)}
macro_rules! lua_tuserdata          {() => (/*#define LUA_TUSERDATA       7*/ 7)}
macro_rules! lua_tthread            {() => (/*#define LUA_TTHREAD    8*/ 8)}

// error codes
macro_rules! lua_ok                 {() => (/*#define LUA_OK     0*/ 0)}
macro_rules! lua_yield              {() => (/*#define #define LUA_YIELD 1*/ 1)}
macro_rules! lua_errrun             {() => (/*#define LUA_ERRRUN     2*/ 2)}
macro_rules! lua_errsyntax          {() => (/*#define LUA_ERRSYNTAX   3*/ 3)}
macro_rules! lua_errmem             {() => (/*#define LUA_ERRMEM 4*/ 4)}
macro_rules! lua_errgcmm            {() => (/*#define LUA_ERRGCMM    5*/ 5)}
macro_rules! lua_errerr             {() => (/*#define LUA_ERRERR    6*/ 6)}
macro_rules! lua_ridx_mainthread    {() => (/*#define LUA_RIDX_MAINTHREAD    1*/ 1)}

// type checking
macro_rules! lua_isfunction {($L:expr, $n:expr)        => {ffi::lua_type($L, $n) == lua_tfunction!()}}
macro_rules! lua_istable{($L:expr, $n:expr)            => {ffi::lua_type($L, $n) == lua_ttable!()}}
macro_rules! lua_islightuserdata{($L:expr, $n:expr)    => {ffi::lua_type($L, $n) == lua_tlightuserdata!()}}
macro_rules! lua_isnil{($L:expr, $n:expr)              => {ffi::lua_type($L, $n) == lua_tnil!()}}
macro_rules! lua_isboolean{($L:expr, $n:expr)          => {ffi::lua_type($L, $n) == lua_tboolean!()}}
macro_rules! lua_isthread{($L:expr, $n:expr)           => {ffi::lua_type($L, $n) == lua_tthread!()}}
macro_rules! lua_isnone{($L:expr, $n:expr)             => {ffi::lua_type($L, $n) == lua_tnone!()}}
macro_rules! lua_isnoneornil{($L:expr, $n:expr)        => {ffi::lua_type($L, $n) <= 0}}

macro_rules! lua_upvalueid {
    () => (
/*
  StkId fi = index2addr(L, fidx);
  switch (ttype(fi)) {
    case LUA_TLCL: {  /* lua closure */
      return *getupvalref(L, fidx, n, NULL);
    }
    case LUA_TCCL: {  /* C closure */
      CClosure *f = clCvalue(fi);
      api_check(L, 1 <= n && n <= f->nupvalues, "invalid upvalue index");
      return &f->upvalue[n - 1];
    }
    default: {
      api_check(L, 0, "closure expected");
      return NULL;
    }
  }
*/
    )
}

macro_rules! luaL_requiref {
    ($lua_State:expr, $modname:expr, $openf:expr, $glb:expr) => ({
/*

        (lua_State *L, const char *modname, lua_CFunction openf, int glb)

          luaL_getsubtable(L, LUA_REGISTRYINDEX, LUA_LOADED_TABLE);
          lua_getfield(L, -1, modname);  // LOADED[modname]
          if (!lua_toboolean(L, -1)) {  // package not already loaded?
            lua_pop(L, 1);  // remove field
            lua_pushcfunction(L, openf);
            lua_pushstring(L, modname);  // argument to open function
            lua_call(L, 1, 1);  // call 'openf' to open module 7
            lua_pushvalue(L, -1);    // make copy of module (call result) 7
            lua_setfield(L, -3, modname);  // LOADED[modname] = module 7
          }
          lua_remove(L, -2);  // remove LOADED table
          if (glb) {
            lua_pushvalue(L, -1);    // copy of module
            lua_setglobal(L, modname);    // _G[modname] = module
          }
*/
    })
}

macro_rules! index2addr {
    ($lua_State:expr, $idx:expr) => ({
/*
        CallInfo *ci = L->ci;
        if (idx > 0) {
            TValue *o = ci->func + idx;
            api_check(L, idx <= ci->top - (ci->func + 1), "unacceptable index");
            if (o >= L->top) return NONVALIDVALUE;
            else return o;
            }
            else if (!ispseudo(idx)) {  /* negative index */
            api_check(L, idx != 0 && -idx <= L->top - (ci->func + 1), "invalid index");
            return L->top + idx;
            }
            else if (idx == LUA_REGISTRYINDEX)
            return &G(L)->l_registry;
            else {  /* upvalues */
            idx = LUA_REGISTRYINDEX - idx;
            api_check(L, idx <= MAXUPVAL + 1, "upvalue index too large");
            if (ttislcf(ci->func))  /* light C function? */
              return NONVALIDVALUE;  /* it has no upvalues */
            else {
              CClosure *func = clCvalue!(ci->func);
              return (idx <= func->nupvalues) ? &func->upvalue[idx-1] : NONVALIDVALUE;
            }
        }
*/
    })
}


macro_rules! lua_tointegerx {
    () => ({

/*
    lua_Integer res;
    const TValue *o = index2addr(L, idx);
    int isnum = tointeger(o, &res);
    if (!isnum)
      res = 0;  /* call to 'tointeger' may change 'n' even if it fails */
    if (pisnum) *pisnum = isnum;
    return res;
*/
    })
}

macro_rules! lua_geti {
    ($lua_State:expr, $i:expr, $n:expr) => (/*
      StkId t;
      const TValue *slot;
      lua_lock($lua_State);
      t = index2addr($lua_State, idx);
      if (luaV_fastget($lua_State, t, n, slot, luaH_getint)) {
        setobj2s($lua_State, L->top, slot);
        api_incr_top($lua_State);
      }
      else {
        setivalue($lua_State->top, n);
        api_incr_top($lua_State);
        luaV_finishget($lua_State, t, $lua_State->top - 1, $lua_State->top - 1, slot);
      }
      lua_unlock(L);
      return ttnov($lua_State->top - 1);
      */
    )
}

macro_rules! luaL_len {
    ($lua_State:expr, $i:expr) => ({

/*
        lua_Integer l;
        int isnum;
        lua_len(L, idx);
        l = lua_tointegerx(L, -1, &isnum);
        if (!isnum)
          luaL_error(L, "object length is not an integer");
        lua_pop(L, 1);  /* remove object */
        return l;
*/
    })
}
