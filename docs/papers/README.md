# Papers

高専プロコン競技部門「ヘキサうどん」の解法で参照した論文・技術資料をまとめています。

> [!IMPORTANT]
> 再配布が許可されていない論文の PDF は commit しないでください。
> 各自で DOI、出版社、著者ページ、所属機関 repository などの正規の配布元から取得し、ローカルの `docs/papers/` に配置してください。
>
> `docs/papers/*.pdf` は `.gitignore` で除外していますが、`.gitignore` の変更や `git add -f` などで誤って PDF を commit・公開しないよう注意してください。

## References

### Adaptive Large Neighborhood Search

#### Røpke & Pisinger (2006)

**Stefan Røpke, David Pisinger.**  
*An Adaptive Large Neighborhood Search Heuristic for the Pickup and Delivery Problem with Time Windows.*  
Transportation Science, 40(4), 455–472, 2006.

- DOI: https://doi.org/10.1287/trsc.1050.0135
- DTU Research Database: https://orbit.dtu.dk/en/publications/an-adaptive-large-neighborhood-search-heuristic-for-the-pickup-an/
- Keywords: ALNS, destroy/repair operator, adaptive operator selection, PDPTW

ALNS の基本となる文献です。複数の destroy / repair operator を用意し、探索中の成績に応じて operator の選択確率を適応的に更新する枠組みを示しています。

「ヘキサうどん」では、現在解の一部を壊して別の割当て・経路を作り直す LNS / ALNS の設計、特に destroy / repair operator の構成を考える際の基礎になります。

#### Santini (2019)

**Alberto Santini.**  
*An Adaptive Large Neighbourhood Search Algorithm for the Orienteering Problem.*  
Expert Systems with Applications, 123, 154–167, 2019.

- DOI: https://doi.org/10.1016/j.eswa.2018.12.050
- Author preprint: https://santini.in/files/papers/santini-2019.pdf
- Keywords: Orienteering Problem, ALNS, clustering

Orienteering Problem に ALNS を適用した文献です。近接する頂点をまとめて扱う operator など、問題の空間的な構造を利用した探索を扱っています。

六角形グリッド上で地理的に近い候補をまとめて変更する neighborhood を設計する際の参考になります。

#### Hammami, Rekik & Coelho (2020)

**Farouk Hammami, Monia Rekik, Leandro C. Coelho.**  
*A hybrid adaptive large neighborhood search heuristic for the team orienteering problem.*  
Computers & Operations Research, 123, 105034, 2020.

- DOI: https://doi.org/10.1016/j.cor.2020.105034
- Keywords: Team Orienteering Problem, HALNS, local search, set packing

ALNS に局所探索と Set Packing を組み合わせた hybrid heuristic です。複数経路を同時に最適化する Team Orienteering Problem を扱っています。

複数エージェントの経路を個別に改善するだけでなく、良い部分経路を蓄積して全体として再構成する考え方が参考になります。

### Orienteering / Team Orienteering

#### Chao, Golden & Wasil (1996)

**I-Ming Chao, Bruce L. Golden, Edward A. Wasil.**  
*The team orienteering problem.*  
European Journal of Operational Research, 88(3), 464–474, 1996.

- DOI: https://doi.org/10.1016/0377-2217(94)00289-4
- Keywords: Team Orienteering Problem, heuristic search

Team Orienteering Problem の代表的な初期文献です。複数の経路に限られた時間を割り当て、訪問地点から得られる総スコアを最大化する問題を扱います。

複数エージェントに「どこを担当させるか」と「各エージェントがどの順序で回るか」を同時に考えるための基本的な問題設定として参考になります。

#### Pěnička, Faigl & Saska (2019)

**Robert Pěnička, Jan Faigl, Martin Saska.**  
*Variable Neighborhood Search for the Set Orienteering Problem and its application to other Orienteering Problem variants.*  
European Journal of Operational Research, 276(3), 816–825, 2019.

- DOI: https://doi.org/10.1016/j.ejor.2019.01.047
- Supporting material: https://mrs.fel.cvut.cz/vns-sop
- Authors' implementation: https://github.com/ctu-mrs/vns-sop
- Keywords: Set Orienteering Problem, VNS, ILP

頂点を set / cluster にまとめ、その集合に属する候補を訪問することで利益を得る Set Orienteering Problem を扱います。

著者らは提案手法の実装も公開しており、repository には VNS-SOP と CPLEX を用いた ILP solver のソースコード、SOP / OPN / DOP の benchmark datasets、論文で用いた計算結果、可視化スクリプトが含まれています。そのため、論文中のアルゴリズムを読むだけでなく、実際の neighborhood 操作やデータ構造、パラメータ設定、評価方法をコードと対応させて確認できます。

「同じ系列に属する候補地点のうち、どこを訪問するか」という構造をモデル化・最適化する際に特に参考になります。また、「ヘキサうどん」向けの VNS / LNS を実装する際には、著者実装を neighborhood 操作や探索戦略を設計するための具体的な参考実装として利用できます。

#### Nguyen, Martinelli, Pham & Hà (2025)

**Tat Dat Nguyen, Rafael Martinelli, Quang Anh Pham, Minh Hoàng Hà.**  
*The set team orienteering problem.*  
European Journal of Operational Research, 321, 75–87, 2025.

- DOI: https://doi.org/10.1016/j.ejor.2024.09.021
- Keywords: Set Team Orienteering Problem, Branch-and-Price, LNS

Set Orienteering Problem を複数車両へ拡張した Set Team Orienteering Problem を扱います。厳密解法として Branch-and-Price、大規模問題向けの heuristic として LNS が提案されています。

「系列（set）」と「複数エージェント（team）」を同時に扱うため、「ヘキサうどん」の問題構造との対応を考えるうえで特に重要な文献です。

## Local files

各自で取得した PDF は、必要に応じて次のように配置してください。

```text
docs/papers/
├── README.md
├── santini-2019.pdf
├── ejor19sop.pdf
└── ...
```

`docs/papers/*.pdf` は `.gitignore` で除外されているため、`git add .` を実行しても PDF は commit されません。

## Adding references

新しい論文を追加するときは、次の情報を README に記録してください。

1. 著者・タイトル・掲載誌・出版年
2. DOI
3. 出版社、著者ページ、機関 repository などの正規の入手先
4. 主なキーワード
5. 「ヘキサうどん」のどの部分に応用できそうか

PDF を入手できる場合でも、PDF 本体は Git に追加せず、各メンバーが正規の配布元から取得する運用とします。

## Copyright

各論文の著作権その他の権利は、それぞれの著者・出版社等の権利者に帰属します。

この repository では論文の書誌情報と正規の入手先のみを共有し、論文 PDF 本体は配布しません。Web 上で無料で閲覧できることと、PDF 自体の再配布が許可されていることは必ずしも同じではありません。各自で正規の配布元から取得した PDF を、Git repository やその他の共有場所へ転載・再配布しないでください。

ライセンスが明示されている資料を別途収録・再配布する場合は、そのライセンス条件に従ってください。