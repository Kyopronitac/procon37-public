# 依存関係とライセンス

- テスタの入口 → [../README.md](../README.md)
- 環境の準備 → [setup.md](setup.md#必要な環境)

## 通常利用と検証の違い

| 用途 | 必要な環境 |
| --- | --- |
| テスタ・ソルバのビルド | Rust / Cargo |
| ビルド済みテスタのGUI | ブラウザ |
| GIF出力 | ブラウザ内で完結。追加ソフト不要 |
| 履歴ZIPの入出力 | Rust実装。外部の圧縮コマンド不要 |
| プロセス境界のテスト | Python 3 |
| 画面のテスト | Node.js |
| GIFの検証 | Node.js、Python 3、Pillow |

- Rust依存の指定 → [tester/Cargo.toml](../Cargo.toml)、[common/Cargo.toml](../../../common/Cargo.toml)
- 解決された依存 → [../../Cargo.lock](../../../Cargo.lock)

## GIFエンコーダー

| 項目 | 内容 |
| --- | --- |
| ライブラリ | [gifenc](https://github.com/mattdesl/gifenc) v1.0.3 |
| 同梱ファイル | [web/vendor/gifenc.js](../../../common/web/vendor/gifenc.js) |
| 取得元 | [v1.0.3のES Module配布ファイル](https://github.com/mattdesl/gifenc/blob/v1.0.3/dist/gifenc.esm.js) |
| ライセンス | MIT / Copyright 2017 Matt DesLauriers |
| 配布ファイルへの変更 | 同梱しないソースマップへの参照コメントだけを削除 |

- ライセンス全文 → [../../common/web/vendor/gifenc.LICENSE.md](../../../common/web/vendor/gifenc.LICENSE.md)

量子化・GIFエンコードはWeb Worker内で実行し、画面描画と分離する。ブラウザ用ファイルもRust実行ファイルに同梱するため、CDNへの接続は不要。

フレーム抽出・Canvas描画・進捗・中断はこのプロジェクトで実装している。GIFテストでは、Pillowの別実装デコーダーを使い、色・フレーム時間・ループ設定を確認する。

- GIFの検証コード → [../tests/gif_test.py](../tests/gif_test.py)

## 履歴ZIP

| ライブラリ | バージョン | 用途 |
| --- | --- | --- |
| [zip](https://docs.rs/zip/8.6.0/zip/) | 8.6.0 | Deflate圧縮によるZIP入出力 |
| [tempfile](https://docs.rs/tempfile/3.27.0/tempfile/) | 3.27.0 | 一時ファイル・取り込み用ディレクトリ |
| [tokio-util](https://docs.rs/tokio-util/0.7.19/tokio_util/io/struct.ReaderStream.html) | 0.7.19 | ZIPのHTTP配信 |

- ZIPの上限・整合検証 → [history.md](history.md#zipの形式と上限)
