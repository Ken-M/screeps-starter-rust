//! Memory アクセス層。
//!
//! 旧 screeps-game-api 0.9 の `MemoryReference`(`.bool()` / `.i32()` / `.string()` /
//! `.set()` / `.del()`)は 0.23 で消滅し、`creep.memory()` は素の `JsValue` を返す。
//! 呼び出し側 (約85箇所) を書き換えずに済むよう、旧 API と同名・同シグネチャの
//! メソッドを `JsValue` に生やす。

use js_sys::Reflect;
use wasm_bindgen::{JsCast, JsValue};

/// `Memory` ルートオブジェクト。旧 `screeps::memory::root()` 相当。
///
/// **毎回グローバルから引き直すこと。** `screeps::memory::ROOT` は
/// wasm-bindgen の `extern static` で、生成コードが `thread_local!` により
/// 初回アクセス時の値を永久にキャッシュする。Screeps エンジンは tick ごとに
/// `Memory` グローバルを差し替えるため、キャッシュされた参照へ書いても
/// エンジンには一切反映されない (＝書き込みが黙って捨てられる)。
/// `ROOT` が deprecated なのはこのため。
pub fn root() -> JsValue {
    Reflect::get(&js_sys::global(), &JsValue::from_str("Memory"))
        .unwrap_or(JsValue::UNDEFINED)
}

pub trait MemoryExt {
    /// キーが無い・bool でない場合は false (旧 API と同じ既定値挙動)。
    fn bool(&self, key: &str) -> bool;
    /// 旧 API の `Result<Option<i32>, _>` を踏襲し、`.unwrap_or(Some(0)).unwrap_or(0)`
    /// という既存の呼び出しチェーンをそのまま生かす。キー欠如は Ok(None)、
    /// 数値以外が入っていたら Err。
    fn i32(&self, key: &str) -> Result<Option<i32>, ()>;
    fn string(&self, key: &str) -> Result<Option<String>, ()>;
    fn set<T: Into<JsValue>>(&self, key: &str, value: T);
    fn del(&self, key: &str);
}

impl MemoryExt for JsValue {
    fn bool(&self, key: &str) -> bool {
        Reflect::get(self, &JsValue::from_str(key))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn i32(&self, key: &str) -> Result<Option<i32>, ()> {
        match Reflect::get(self, &JsValue::from_str(key)) {
            Ok(v) if v.is_undefined() || v.is_null() => Ok(None),
            Ok(v) => v.as_f64().map(|n| Some(n as i32)).ok_or(()),
            Err(_) => Err(()),
        }
    }

    fn string(&self, key: &str) -> Result<Option<String>, ()> {
        match Reflect::get(self, &JsValue::from_str(key)) {
            Ok(v) if v.is_undefined() || v.is_null() => Ok(None),
            Ok(v) => v.as_string().map(Some).ok_or(()),
            Err(_) => Err(()),
        }
    }

    fn set<T: Into<JsValue>>(&self, key: &str, value: T) {
        let _ = Reflect::set(self, &JsValue::from_str(key), &value.into());
    }

    fn del(&self, key: &str) {
        let _ = Reflect::delete_property(
            self.unchecked_ref::<js_sys::Object>(),
            &JsValue::from_str(key),
        );
    }
}
