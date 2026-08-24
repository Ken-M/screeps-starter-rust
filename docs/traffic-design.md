# 交通管理 (swap プロトコル) 設計書

対象: `migrate-api-0.23` / 全ロールの移動
状態: **設計のみ (未実装)**
作成: 2026-08-25

---

## 1. 背景と目的

### 1.1 観測された問題

1. **道路上の正面衝突**: 一本道の道路で対向 creep とぶつかると、片方が道路から
   降りて迂回する。道路特化 body (MOVE 半減) は平地で倍の fatigue を払うため、
   すれ違いのたびに 1〜2 tick を損する。
2. **待機・作業中の creep による通せんぼ**: 建設・修理で立ち止まる worker や
   待機 hauler が通路を塞ぎ、後続が迂回するか詰まる。

### 1.2 これまでの対処と限界

| 対処 | 結果 |
|---|---|
| 味方 creep をコスト8で迂回誘導 (現行) | 動くが、対向のたびに道路から降りる (問題1が残る) |
| 道路上の creep をコスト0扱い (7ecc33f、撤回済み) | 対向は直進すれ違いで解決するが、**静止 creep に永遠にぶつかる livelock** を誘発。progress/tick -71% の回帰 (f3cfcf3 で撤回) |
| 着席 creep (miner / 指定席 upgrader) を通行不可 0xff (現行) | 「退かない相手」は正しくモデル化された。ただし「今 tick 動くか分からない相手」は表現できない |

コストマトリクスは「そのマスに今 creep がいるか」しか表現できず、
**「その creep が今 tick 動くのか」という意図**を知らないのが根本原因。
意図は同じ tick 内に全 creep のロール処理を回す我々自身が持っている。
これを突き合わせるのが本設計。

### 1.3 エンジン仕様の前提 (設計の土台)

Screeps 公式エンジンの move 解決は tick 末にまとめて行われ、以下が成立する:

- **明け渡し追従**: creep A が「同 tick に別マスへ移動する creep B」のマスへ
  move すると、B の移動が成功する限り A も成功する (数珠つなぎの隊列も可)。
- **相互スワップ**: A→B のマス、B→A のマス、を同 tick に発行すると両方成功する。
- **循環**: 3体以上の巡回 (A→B→C→A) も同様に成立する。
- **失敗の安全性**: 明け渡し側が動けなければ追従側も止まるだけ (すり抜けや
  重なりは起きない)。
- **同一マスへの競合**: 複数 creep が同じ空きマスへ move すると勝者は
  エンジン依存。**自軍同士の競合は解決器側で事前に排除する** (§4.3)。

つまり「両者が move を発行しさえすれば」すれ違いは無料で成立する。
問題は経路探索が相手のマスを避けてしまい move を発行しないことにある。

---

## 2. 設計の核

**移動を「即時発行」から「意図の登録 + tick 末の一括解決」に変える。**

```
現行:   role処理 → find_path → move_by_search_result → 即 creep.move発行
新方式: role処理 → find_path → traffic::request_move (意図を登録するだけ)
        creep_loop 終了後 → traffic::resolve() が全意図を突き合わせ、
        許可した移動だけ creep.move_direction で一括発行
```

- ロール側のコードは `move_by_search_result(creep, &res)` を
  `traffic::request_move(creep, &res)` に置き換えるだけ (18箇所、同シグネチャ)。
- 解決器は「隊列」「スワップ」「押し出し (shove)」「優先度」を一元処理する。
- 経路探索側は将来、道路上の味方コストを下げて「直進してすれ違う」経路を
  作れるようになる (livelock は解決器が防ぐ。§6 Phase 3)。

---

## 3. データ構造と API

新モジュール `src/util/traffic.rs`。

```rust
/// 1件の移動意図。
struct Intent {
    creep: Creep,        // 発行者 (JsValue 参照。tick 内でのみ有効)
    from: Position,      // 現在位置
    to: Position,        // 次の1マス (経路の先頭。1 tick に1マスしか進めない)
    priority: u8,        // 競合時の優先度 (§4.3)
}

thread_local! {
    /// tick 内で収集する意図。COLONY_CACHE と同じ tick 内キャッシュ方式。
    static INTENTS: RefCell<Vec<Intent>> = ...;
}

/// tick 先頭で呼ぶ (lib.rs の clear_colony_cache と同じ場所)。
pub fn clear();

/// move_by_search_result の置き換え。経路の先頭1マスを意図として登録する。
/// 優先度は creep のロールから内部で決める (呼び出し側の変更を最小に)。
/// fatigue > 0 の creep は登録しない (どうせ動けない = 静止扱い)。
pub fn request_move(creep: &Creep, res: &SearchResults);

/// creep_loop の末尾で呼ぶ。意図を解決し、許可分だけ move_direction を発行。
pub fn resolve();
```

### 3.1 意図の座標抽出

`SearchResults::path()` の先頭要素が次のマス。`from.get_direction_to(to)` で
Direction を得て `creep.move_direction(dir)` を発行する。
`move_by_path` は使わない (発行タイミングを解決器が握るため)。

### 3.2 優先度表

競合 (同じ空きマスの取り合い・押し出しの要否) はこの順で解決する。
数値は間隔を空けて定義し、将来の挿入余地を残す。

| priority | 対象 | 理由 |
|---|---|---|
| 100 | defender | 防衛は経済より優先 |
| 60 | hauler | 物流が止まると全員が止まる (実測済み) |
| 50 | miner | 席に着くまでの移動。採取開始の遅れは収入の直接損 |
| 40 | upgrader | 席に着くまでの移動 |
| 30 | worker / builder / repairer | 汎用作業 |
| 10 | shove (解決器が生成する押し出し) | 本人の意図ではないので最弱 |

### 3.3 静止 creep の分類 (shove 可否)

| 分類 | 判定 | 扱い |
|---|---|---|
| 着席 (parked) | miner が source 隣 / upgrader が claim 席の上 (pathing.rs の 0xff 判定と同一。**共通ヘルパー `is_parked(creep)` に切り出して両者で使う**) | 押し出し不可。経路上も 0xff (現行どおり) |
| 硬直 (fatigued) | `creep.fatigue() > 0` | 今 tick は動けない。押し出し不可、追従も不可 |
| 遊休 (idle) | 意図未登録・上記以外 | **押し出し可** (§4.2) |

---

## 4. 解決アルゴリズム

### 4.1 チェーン追跡

意図を優先度降順に処理する。各意図 A について行き先の占有チェーンを辿る:

```
follow(A):
  visited = {A}
  cur = A
  loop:
    to = cur.to のマスの占有者
    ├ 誰もいない (かつ予約もない) → チェーン全員を許可
    ├ 敵 creep              → チェーン全員を却下 (待つ。経路は次 tick 引き直し)
    ├ 味方 parked / fatigued → チェーン全員を却下
    ├ 味方 idle (shove可)    → shove を試行 (§4.2)。成功なら全員許可、失敗なら却下
    ├ 味方 with 意図:
    │   ├ すでに許可済み   → チェーン全員を許可 (隊列: 明け渡し追従)
    │   ├ すでに却下済み   → チェーン全員を却下
    │   ├ visited に含まれる → 循環 (swap を含む) → チェーン全員を許可
    │   └ 未処理           → cur = その意図として継続 (visited に追加)
```

- 各 creep の許可/却下は memo 化するので計算量は O(意図数)。
- **swap はこの規則から自然に導かれる**: A→B のマス、B→A のマスなら、
  A の追跡が B に到達し、B の行き先 (= A のマス) が visited に含まれるので
  循環として両方許可 → 両方 move 発行 → エンジンがすれ違いを成立させる。

### 4.2 押し出し (shove)

遊休 creep C の退避先は以下の条件で選ぶ:

1. C の隣接マスで `is_walkable_tile` が真
2. 現在占有なし、かつ解決済み意図の行き先 (予約) でもない
3. 優先順: 非道路マス > 道路マス (幹線を新たに塞がない)
4. 候補なし → shove 失敗 (元の意図は却下)

shove は深さ1まで (押された creep がさらに押すことはしない)。
連鎖 shove は解決が複雑になる割に、深さ1で塞がる状況は
「席・レーン設計 (claimable_seats) で構造的に防ぐ」のが本筋のため。

shove された creep への影響: 次 tick に自分の仕事の経路を引き直すだけ。
作業対象への射程が外れる可能性はあるが、build/repair/upgrade は射程3なので
1マスの退避で外れることは稀。

### 4.3 同一マス競合の事前排除

許可した意図の行き先は「予約」として記録し、以降の意図・shove の行き先候補
から除外する。優先度降順に処理しているので、高優先度が必ず勝つ。
これによりエンジン依存の勝敗 (§1.3) を自軍内から排除する。

### 4.4 却下された意図

何もしない (その tick は待ち)。次 tick にロール処理が経路を引き直す。
- 相手が動く creep なら次 tick には空く (1 tick 待ちで済む)。
- 相手が parked なら経路探索の 0xff が迂回路を作る (現行どおり)。
- 「同じ相手に3 tick 連続で却下」のような livelock 検知は Phase 2 の
  計測で必要性を判断する (意図的に入れない: 複雑さ先行を避ける)。

---

## 5. エッジケースと決め事

| ケース | 扱い |
|---|---|
| fatigue > 0 の発行者 | request_move で登録拒否 (動けないのに意図を持つと隊列判定を狂わせる) |
| 部屋境界 (exit マス) | 部屋ごとに独立して解決する。exit への移動は通常マスと同じ。部屋間の乗り継ぎはエンジン任せ (現行どおり) |
| 敵 creep のマス | 却下 (追わない)。攻撃判断は attacker_routine の仕事 |
| spawning 中の creep | creep_loop が既にスキップしており意図を持たない。占有としては「遊休」だが shove 不可 (動けない) → fatigued と同じ却下扱い |
| pull / 牽引 | 未使用のため対象外。導入時は本設計の「隊列」に牽引ペアを追加する |
| 意図の二重登録 | 同一 creep の2回目の request_move は上書き (最後の意図が勝つ)。現行コードは1 creep 1移動なので通常発生しない |

---

## 6. 段階的ロールアウト

一気に入れて -71% 級の回帰を繰り返さないため、3段階に分けて
各段階でベンチ (settle 8 / intervals 8) を通す。

### Phase 1: 配管の置き換え (挙動は不変)
- traffic.rs 新設。request_move は登録し、resolve は**全許可**で
  move_direction を発行するだけ (競合解決なし)。
- 18箇所の呼び出しを置き換え。move_by_search_result は traffic 経由の
  実装に差し替えて削除。
- 検証: ベンチで progress/tick・CPU が変わらないこと。
  (move_by_path → move_direction の等価性確認がこの段階の本題)

### Phase 2: 解決器の有効化
- チェーン追跡・swap/循環・予約・優先度・shove を有効化。
- 検証: ベンチ + 「同一 creep が同一マスに3 tick 以上滞留した回数」を
  debug ログで計測し、Phase 1 比で減ること。

### Phase 3: 経路コストの最適化 (元の目的の回収)
- 道路上の味方 creep コストを 8 → 2 に下げる (0 にはしない: 実際に
  塞がっている可能性への微ペナルティ)。経路が対向 creep を避けずに
  直進し、解決器が swap で成立させる。
- 静止 creep への直進は解決器が shove または却下で処理するので、
  7ecc33f の livelock は再発しない。
- 検証: ベンチ + 対向すれ違い時の道路離脱が消えること (目視)。

### 撤退基準
各 Phase でベンチが ⚠ (進捗 -25% 超 / CPU +25% 超 / backlog 倍増) なら
直前 Phase へ戻す。Phase 単位のコミットにして revert 可能にしておく。

---

## 7. テスト戦略

解決器の中核を game API から分離した純ロジックにする:

```rust
/// game API 非依存の意図表現。単体テストはこちらを使う。
struct IntentData { id: usize, from: (u8, u8), to: (u8, u8), priority: u8 }
enum Occupant { Parked, Fatigued, Idle, Moving(usize /* intent id */), Enemy }

/// 純関数: 意図と占有状態から、許可する意図 id と shove の一覧を返す。
fn resolve_intents(
    intents: &[IntentData],
    occupied: &HashMap<(u8, u8), Occupant>,
    walkable: &dyn Fn((u8, u8)) -> bool,
) -> Resolution { granted: Vec<usize>, shoves: Vec<(usize /*creep*/, (u8, u8))>, denied: Vec<usize> }
```

必須テストケース (既存テストの日本語命名に合わせる):

1. 空きマスへの単独移動は許可される
2. 対向2体のswapは両方許可される
3. 3体の循環は全員許可される
4. 隊列 (前が動けば後ろも動く) は全員許可される
5. 先頭が着席creepなら隊列全員が却下される
6. 遊休creepは空きマスへ押し出され_元の意図は許可される
7. 押し出し先が無ければ却下される
8. 同一マスを取り合ったら優先度の高い方が勝つ
9. 押し出し先は許可済み意図の行き先と重複しない
10. fatigue中のcreepは押し出せず隊列も止まる

wasm 側 (traffic.rs の game API 接着部) は薄く保ち、実機はベンチで検証する。

---

## 8. CPU への影響見積り

- 意図収集: Vec push のみ。無視できる。
- 解決: creep 数 n (現在 16〜17) に対し O(n) のチェーン追跡 + HashMap 数個。
  wasm 内で完結し JS 呼び出しなし。< 0.1 CPU/tick 見込み。
- move 発行: 従来も move_by_path で 1 creep 1 JS 呼び出しだったので同数。
- shove 発生時のみ +1 発行/件。
- 相殺要因: 却下により無駄な move 発行 (ERR_NO_BODYPART 相当の空振り) が減る。
  move 1発行 = 0.2 CPU (intent cost) なので、却下が tick あたり数件あれば
  解決器のコストを上回る節約になる。

---

## 9. 実装しないと決めたこと (YAGNI)

- 連鎖 shove (深さ2以上) — 席・レーンの構造設計で防ぐ方が安い
- 経路全体の予約 (時空間予約テーブル) — 1マス先の解決で十分。RCL が上がり
  creep 数が数十になったら再検討
- 部屋間をまたぐ意図の突き合わせ — 単一部屋運用の間は不要
- livelock 検知 (n tick 連続却下で強制迂回) — Phase 2 の計測で必要なら追加
