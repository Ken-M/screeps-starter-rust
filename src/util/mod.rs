//! ユーティリティ群。責務別のサブモジュールに分かれる。
//!
//! - `cache`     — tick 単位のキャッシュ (find 結果・地形・仕事サマリ)
//! - `pathing`   — 経路探索とコストマトリクス (静的層/動的層)
//! - `predicates`— 建造物の状態判定と資源の分類
//! - `stats`     — 部屋単位の HP / 建設進捗統計
//! - `finders`   — 最寄り対象の探索
//! - `traffic`   — 移動意図の収集と一括解決 (docs/traffic-design.md)
//!
//! 呼び出し側は従来どおり `use crate::util::*;` で全部が見える。

mod cache;
mod finders;
mod pathing;
mod predicates;
mod stats;
mod traffic;

pub use cache::*;
pub use finders::*;
pub use pathing::*;
pub use predicates::*;
pub use stats::*;
pub use traffic::*;
