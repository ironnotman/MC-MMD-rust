# Stage 6差分 — MMDラグドールの即時崩壊対策と接触判定

対象は **Stage 5のパッチが適用済み**の `ragdoll-preview` ブランチです。

## 何が変わる？

1. 骨追従の人体剛体を動的化した際に、人体剛体同士の初期重なりが激しく押し合わないようにします。
2. ラグドールの拘束計算を強化します。人体関節の角度制限はまだ暫定的で、完成版ではありません。
3. プレイヤー・生存モブの当たり判定を、死体周辺から最大16体まで毎クライアントtick収集し、MMD Bullet物理世界に送り込みます。近づいて接触すると死体を押せるようにする試作です。

**注意:** これは実機での改善を確認する前のソース更新です。元のStage 5ビルドを必ずバックアップしてください。Physics ModのPhysXを直接操作しているわけではなく、死体の物理演算はMmdSkin側Bulletのままです。動作はクライアント側だけで、マルチプレイ中に他者と状態同期は行われません。

## Windows PowerShellで更新

前回ビルドに使った `MC-MMD-rust` ディレクトリをPowerShellで開きます。

```powershell
git status --short
git branch --show-current
```

未コミットの別の作業がなければ、現在のブランチが `ragdoll-preview` であることを確認します。必要なら `git switch ragdoll-preview` してください。

次の `C:\path\to\...` をダウンロードした差分パッチの実際のパスに置き換えます。

```powershell
git apply --check "C:\path\to\MmdSkin-ragdoll-stage6.patch"
git apply "C:\path\to\MmdSkin-ragdoll-stage6.patch"
git add -A
git commit -m "Stabilize ragdoll and enable actor contact"
git push
```

GitHubの **Actions** でWindows NeoForgeビルドを実行・確認します（ワークフローの設定によっては手動の `Run workflow` が必要です）。緑色の成功表示が出たら新しいアーティファクトをダウンロードし、検証用Minecraftプロファイル内でStage 5 JARと**置き換えて**試します。新旧JARを同時に `mods` へ入れないでください。

## テストしてほしいこと

1. **死亡直後**: 別々のPMXモデルでも、即座に全身が折り畳まれる現象が減ったか。
2. **地形**: 平地・階段・壁への接触が以前どおり機能するか。
3. **プレイヤー接触**: 死体へ歩いて近づいて接触し、揺れる・移動するか。次に横から押してみる。
4. **モブ接触**: 生きているゾンビや村人が死体を通過するときに反応するか。
5. **安定性**: 複数死体を出してもラグや突然の大きな跳ね上がりがないか。

結果は「①骨格 ②地形 ③プレイヤー ④モブ ⑤その他」の順に A/B/C や短い動画で報告してもらえれば次に絞り込みやすいです。

## 検証状況

- 元のStage 5はユーザー側GitHub ActionsでWindowsビルド成功、ゲーム内で地形との接触を確認済み。
- Stage 6は既存の `1.21.1` ソースに重ねて差分検査を行い、Bullet C++ラッパーの構文確認が通過。
- この環境にRustツールチェーンがないため、Stage 6のRust/Java統合ビルドと新規回帰テスト、およびゲーム内動作の確認は**未実施**。Windows Actionsビルドでの確認が必要です。
