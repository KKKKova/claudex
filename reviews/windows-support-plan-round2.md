---
type: plan-review
slug: windows-support
round: 2
commit: 未コミット（/docs/ local-only 運用。working tree の下記パスを直接参照）
targets:
  - docs/specs/windows-support/plan.md
acceptance: plan.md 内「差分spec」節の受入基準 AC-1〜AC-5（中量経路のため requirements.md なし）
---

前回指摘への対応一覧:

- [required] T002 の CI 反復が成立しない → 対処案(a)を採用。T002 手順4に「push 後 `gh pr create --repo KKKKova/claudex --base main --head fix/windows-support` で fork main 宛 PR を開き、以降 pull_request トリガーで反復する」ことと理由（ci.yml トリガーが push:main / pull_request:main のみ）を明記。手順5も PR マージ（`gh pr merge --merge`、T001 とマージコミット方式で統一）に固定。
- [nit] release.yml の Linux 専用ステップは既にガード済み → 手順2を「既存ガード（"Install system dependencies (Linux)" の `if: runner.os == 'Linux'`、cross install の `if: matrix.cross`）が Windows ランナーで走らないことの確認のみ行う」に修正。
- [nit] launch.rs の実パス → 差分spec の該当箇所を `src/process/launch.rs` ほかフルパス表記に修正。
