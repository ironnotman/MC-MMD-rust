# MmdSkin 1.21.1 / NeoForge — MMDラグドール試作 Stage 5（死体保持）

対象: Minecraft Java 1.21.1、NeoForge、MmdSkin 1.0.5（公式 `1.21.1` ブランチ）。

**これは試作ソースです。ZIPやPATCHそのものをMinecraftの `mods` フォルダへ入れても動きません。**
Windows用の改修済みネイティブDLLとNeoForge JARをセットでビルドする必要があります。

## この版で追加したもの

- PMX適用モブとMMDプレイヤーが死亡した際に、MmdSkinの死亡VMDを置き換えてBullet3による骨格ラグドールを開始します。
- 最後の生存ポーズから始め、既存の剛体のうち「ボーン追従型」を質量付きの動的剛体へ切り替えます。
- PMXの親子ボーンをたどって、元々ない胴体・頭・手足の関節を追加生成します。
- 髪・衣装・尻尾など元々のPMX剛体とジョイントを同じBullet世界に維持します。
- 死亡ラグドール開始時に、エンティティの周辺の**読み込み済みMinecraftブロックの衝突形状を取得**し、Bulletの静的剛体として登録します。通常の立方体に加え、階段・ハーフブロック・フェンスなどの複数の衝突箱を持つ形状にも対応する設計です。
- ブロックはエンティティを中心に水平方向3ブロック、上下方向およそ4ブロックの範囲から採取します（最大256衝突箱）。PMXの縮尺、エンティティの向き、座標系のZ軸反転を考慮してMMDの物理世界へ変換します。
- 地形形状の取得・登録に失敗した場合は、暫定的にMMDモデル原点の高さ0へ追加した水平床を使用します。正常に地形を登録できた場合は水平床を取り除きます。
- **Stage C / 選択A: Physics Modの死体保持時間に同期**。死亡時に使っていたMMDモデルのBullet物理状態を「死体専用キャッシュ」へ移し、元のMinecraftモブが消えた後も、NeoForgeのワールド描画イベントでMMD死体だけを描画します。ネイティブモデルを読み直さないため、死亡時の骨格の姿勢を引き継ぎます。
- Physics Mod 3.0.35が導入されていれば、モブ別の`lifetime`と`lifetimeVariance`（基本秒数＋ランダム追加秒数）、消滅アニメーションの最低時間を死亡時に取得します。導入されていない場合は暫定で4秒を使用します。**Physics Modと同じ設定・分布を使いますが、ランダム値は別途抽選するため、2つの死体の秒数が毎回完全一致する意味ではありません。**
- Physics Modの`mobRagdollLimit`（同時死体数の上限）も取得し、MMD側で上限を超えた場合は古い死体から解放します。0ならMMD死体も保持しません。
- 死体を元のモデルキーから切り離すため、プレイヤーが復活すると**生存者は新しいモデル**を使用し、死亡者のMMD死体は設定時間まで残ります。生存時にラグドールを解除する既存処理も維持します。
- 一人称視点では本人のモデルが描画されないことがあるため、死亡時にクライアントtick側でも選択中のMMDモデルを確認し、必要なら死体へ移行します。
- 設定時間が過ぎた死体のモデル・ネイティブ物理・描画資源は即時解放します。ワールド切り替え／ログアウトでも破棄します。
- 物理を持たないPMX、必要な固定剛体が少ないPMX、VRMでは通常の死亡アニメーションへフォールバックします。
- **Physics Mod 3.0.35 (NeoForge 1.21.1) 向け互換処理**: MMDが実際に表示対象となっているPMX/PMDモブ・プレイヤーについて、Physics Modの`blockifyEntity`を止め、バニラモデルのラグドール生成と元エンティティの即時削除を防ぎます。非MMDモブやVRMはPhysics Modに任せます。
- Physics Mod未導入時は任意対象Mixin（`@Pseudo`）が適用されず、単体使用も可能です。

## まだできないもの・注意

- **地形との衝突は死亡開始時のスナップショット**です。倒れた後のブロック破壊・設置、動く足場などに即時追従しません。液体・他のエンティティ・ワールド境界や未読み込みチャンクは物理的な衝突対象にしていません。
- **探索範囲と衝突箱数には制限**があります。大きなMMDモデル、密集した特殊形状や遠くの壁では衝突が欠けることがあります。モデルのオフセットやVR時の位置・向きは未検証です。
- **死体の保持はクライアント側だけ**です。保存・ネットワーク同期・ワールドをまたぐ永続化は実装していません。死体用のサーバー側エンティティは生成せず、描画は死亡地点から最大128ブロックまでです。
- **Physics Modの消滅エフェクトそのものは再現していません**。この試作は消滅までの「寿命」を同期します。フェード・粒子・効果音・物理結果はPhysics Modと同一にはなりません。
- **Physics Mod互換処理はバージョン限定**: 提供された3.0.35 JARの`net.diebuddies.physics.PhysicsMod.blockifyEntity(Level, LivingEntity)`を直接対象にします。別バージョンで内部構造が変わると抑制できない可能性があります。起動・実機テストはまだ未実施です。
- MMD側の描画距離・同時モデル数制限によりMMDが選ばれない場合は、Physics Modのバニララグドールが従来通り出ます。
- PMX/PMDでも必要な剛体を持たないモデルは、MMD側で通常の死亡アニメーションにフォールバックします。互換処理でPhysics Modを抑止するケースがあるため、十分な人体剛体を持つモデルで試してください。
- すべてのPMXで安定するとは限りません。3個以上のボーン追従剛体が必要で、関節可動範囲は現在共通値です。指、足首など元PMXに剛体がなければ細かくは曲がりません。
- ネイティブDLLを変更しているため、**元のMmdSkin 1.0.5のJARへJavaクラスだけ上書きしても動きません。**
- ユーザー提供PMXやテクスチャは同梱していません。

## Windows x64版をビルドする方法

### 方法1: GitHub Actions（Windowsでの環境構築が少ない）

1. [公式MmdSkinソース](https://github.com/shiroha-233/MC-MMD-rust)をforkし、ブランチ `1.21.1` をベースに作業ブランチを作ります。
2. 提供した `MmdSkin-ragdoll-preview.patch` をそのソースへ `git apply MmdSkin-ragdoll-preview.patch` で適用し、変更をcommitしてforkへpushします。
3. リポジトリの **Actions** → **MMD Ragdoll - Windows NeoForge preview** → **Run workflow** を実行します。
4. 完了後にArtifactsから `mmdskin-ragdoll-neoforge-1.21.1-windows-x64` をダウンロードし、NeoForge JARを取り出します。
5. 元のMmdSkin JARをバックアップし、同時には入れず、新しいJARに置き換えます。

### 方法2: Windowsでローカルビルド

JDK 21、Rust stable、Visual Studio C++ Build Tools（MSVC）、Gitが必要です。`build-ragdoll-windows.ps1` はビルドと成果物の検査（DLLの一致・互換Mixinの同梱）を自動で行います。

```powershell
git clone --depth 1 --branch 1.21.1 --recurse-submodules https://github.com/shiroha-233/MC-MMD-rust.git
cd MC-MMD-rust
git apply C:\path\to\MmdSkin-ragdoll-preview.patch
.\build-ragdoll-windows.ps1
```

`neoforge/build/libs/` のJARのうち、`-dev` と `-sources` が付かないものを使用します。
PowerShellをコマンドプロンプトから起動する場合は `powershell -ExecutionPolicy Bypass -File .\build-ragdoll-windows.ps1` で実行できます（スクリプトの内容を確認してから実行してください）。MSVCが見つからない場合は、Visual StudioのC++ Build Toolsをセットアップしてください。

## 推奨テスト手順

1. **新しい検証用のMinecraftプロファイル・検証用ワールド**で作業してください。先にワールド・設定のバックアップを残してください。
2. MmdSkin改修版とその前提だけを入れ、MMDモデルを割り当てたモブを倒してください。原版モデルや既存の死亡VMDではなく、関節付きのMMDモデルが物理で倒れるか確認します。
3. 次にPhysics Mod 3.0.35を追加し、**MMDのないモブ**と**MMDを適用したモブ**を別々に倒します。前者はPhysics Modの通常ラグドール、後者はMMDモデル1体のみの物理死亡が期待されます。
4. **地形テスト**として平地→ハーフブロック→階段→壁際→崖際の順に試します。死体の腰・手足が地面をすり抜けないか、階段・段差で接地位置が変わるか、壁にぶつかるかを確認してください。
5. **Stage Cの死体保持テスト**: Physics ModのLifetimeをまず4秒、次に8秒などに変更し、それぞれ設定時間の間、元のモブが消えた後もMMD死体が残るかを確認します。Lifetime Varianceを0→数秒に変えた場合のばらつき、モブ別の上書き設定、死体の同時数上限0/1/複数も確認してください。
6. **プレイヤー死亡／復活テスト**: 一人称と三人称で死亡させ、復活したプレイヤーのMMDモデルが正常動作しつつ古い死体が保持されるか確認します。続けて複数回死亡、ワールド切り替え、ログアウトも試してください。
7. 結果を「MMDモデル自身が動くか／地形に接触するか／原版の死体が重複するか／保持秒数・同時数・復活が正常か／ログにエラーがあるか」に分けて記録します。
8. 問題があれば `logs/latest.log` の `[MMD ragdoll]` 付近と、必要なら `crash-reports` の該当レポートを共有してください。Rustログでは地形の衝突箱数を、Javaログでは保持tick数・設定読取のフォールバックを確認できます。

## この場で行った検証

- Rust `cargo check --lib` : 成功。
- Rust `cargo test --lib ragdoll` : PMXの親ボーン探索・実物PMXスモークテストと、地形箱の座標変換・Bulletでの落下接地テストが成功。
- Rust `cargo test --lib` : 全88件中87件成功、1件は既存のignoredテスト。
- 提供PMXでは、ボーン追従16剛体があり、追加関節14個を生成。40フレーム進めて剛体が実際に移動し、動的ボーンに非有限座標が発生しないことを確認。
- 提供PMX `ver1.03.pmx` の物理世界へ衝突箱1個を登録する試験が成功。別途Bulletの動的球を落下させ、実際にブロック相当の床で停止することを確認。
- Windows用JARの実機起動テストは未実施。こちらの環境でJavaビルド依存配信先へのGradle接続がブロックされ、最終JARをコンパイルできていません。**Minecraftの実ブロックへの衝突はゲーム内では未検証です。**
- Windows x64用ビルド・検査スクリプトを追加。最新のDLLとJAR内DLLのSHA-256一致、およびPhysics Mod互換Mixin同梱をチェックします。ただしWindows上でのスクリプト実行自体は未検証です。
- Physics Mod 3.0.35本体JARのバイトコードを確認し、`blockifyEntity`が死体生成の後に元のLivingEntityへ`remove(DISCARDED)`を呼ぶことを確認。MMD対象のみエントリポイントでキャンセルするNeoForge互換Mixinを追加しました（ただしJavaコンパイル／実機動作は未検証）。
- Physics Mod 3.0.35の`ConfigMobs.getMobSetting(Entity)`、`MobSetting.getLifetime()`、`getLifetimeVariance()`、`getAnimation().speed`、`ConfigClient.mobRagdollLimit`をバイトコードで確認し、Java側の寿命計算へ反映しました。寿命計算のJUnitテストを追加しましたが、Java依存解決の制約により未実行です。
- 死体専用キャッシュ・tickによる物理更新・通常のエンティティ描画とは独立した死体描画イベントを追加しました。**Java実機での描画・寿命テストは未実施**なので、設定画面でのLifetimeを変えながらの確認を優先してください。

### 補足

MmdSkin自体の既存Bullet3物理を使います。Physics ModのPhysXエンジンへMMDの頂点を移植したわけではないため、Physics Modの物理設定をMMDラグドールへそのまま適用する仕様ではありません。

変更元: [shiroha-233/MC-MMD-rust `1.21.1` ブランチ](https://github.com/shiroha-233/MC-MMD-rust/tree/1.21.1)。元ソースのライセンスに従って利用してください。
