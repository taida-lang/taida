# `taida-lang/crypto` API リファレンス

`taida-lang/crypto` は暗号系プリミティブを提供するコア同梱パッケージ
です。現行公開 surface は SHA-256 ハッシュのみです。

```taida
>>> taida-lang/crypto => @(sha256)
```

`taida-lang/crypto` はプリリュード互換ではなく、利用するには必ず
明示的なインポートが必要です。インポートせずに `sha256` を呼ぶことは
できません。

---

## 1. ハッシュ

### 1.1 `sha256`

```
sha256(value: Str) -> Str
sha256(value: Bytes) -> Str
```

入力の SHA-256 ダイジェストを小文字 16 進文字列 (64 文字) で返します。

- `Str` を渡した場合は UTF-8 バイト列としてハッシュを計算します。
- `Bytes` を渡した場合はバイト列をそのままハッシュします。
- 空入力 (`""` または空 `Bytes`) のハッシュは
  `"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"` です。
- 戻り値は常に 64 文字の小文字 16 進ダイジェストです。大文字を含むこと
  はありません。

```taida
sha256("hello")
// "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"

sha256(@(@(72, 105)))     // Bytes 形式 (= "Hi")
// "f1945cd6c19e56b3c1c78943ef5ec18116907a4ca1efc40c0e93..."
```

---

## 2. バックエンド対応

| バックエンド | 対応範囲 |
|--------------|----------|
| インタプリタ | 対応 |
| ネイティブ | 対応 |
| JS | 対応 |
| WASM (`wasm-min` / `wasm-wasi` / `wasm-edge`) | 利用不可 |
| WASM (`wasm-full`) | 対応 |

`wasm-min` / `wasm-wasi` / `wasm-edge` プロファイルでは
`taida-lang/crypto` のインポート自体がコンパイル時に拒否されます。
詳細は [`docs/reference/wasm_profiles.md`](../reference/wasm_profiles.md)
を参照してください。

---

## 3. 将来の拡張

HMAC、KDF (PBKDF2 / Argon2)、安全な乱数生成、署名検証などは
`taida-lang/crypto` の将来的な拡張領域として予約されています。
現時点の公開 surface は本書に記載した `sha256` のみです。

---

## 関連リファレンス

- [`bundled_packages.md`](bundled_packages.md) — コア同梱パッケージの入口
- [`docs/reference/standard_library.md`](../reference/standard_library.md) — プリリュード API と関連型コンストラクタ
