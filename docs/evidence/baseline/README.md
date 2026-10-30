# Baseline evidence: herdr / hc behavior this runtime replaces

These are redacted copies of the isolated smoke transcripts the work plan cites
(`.omo/plans/ovis-terminal-v1.md`). They were recorded on 2026-10-30 against
herdr 0.9.5 and herdr-core (`hc`) 0.1.14 in throwaway sessions. Every later PR
cites a `Before` line from these files (path plus line number) or states
`absent`.

Redaction replaces the host name with `<host>`, the home directory with
`<home>`, the run root `/tmp/ulw-...` with `<run-root>` and the user name with
`<user>`. Substitutions are per line, so line numbers in the copies match the
originals. `sha256 (source)` is the digest of the original file at its source
path; `sha256 (copy)` is the digest of the committed copy.

| Copy | Source path | Lines | RT observations citing it | sha256 (source) | sha256 (copy) |
|---|---|---|---|---|---|
| [smoke/attempt2-herdr-h03-workspace.log](smoke/attempt2-herdr-h03-workspace.log) | `<run-root>/evidence/smoke/attempt2-herdr-h03-workspace.log` | 19 | RT-HD-05 | `15c0316200656897255f0ec982843cfbd7074ce180797b89c4d6fa747e142386` | `160bbc09553581f5933bd8d9e8677b9c7c5a123caabf34afd244af480264a073` |
| [smoke/attempt2-herdr-h05-process-info-trap.log](smoke/attempt2-herdr-h05-process-info-trap.log) | `<run-root>/evidence/smoke/attempt2-herdr-h05-process-info-trap.log` | 23 | RT-HD-02, RT-HD-03, RT-HD-04 | `d1151c5f45bce3809db8eb4e01453e757bcfa22b55d652919a96b44223ed56b0` | `17b892b65ad7f31b100340e2ffaca03977af871d1b3452f6a42bdc84ecb7e731` |
| [smoke/hc-s02-run-ls.log](smoke/hc-s02-run-ls.log) | `<run-root>/evidence/smoke/hc-s02-run-ls.log` | 21 | RT-HC-08 | `1fbf0179a006f0b34b45f377eccac8f406bee3cd35a539e5280c0577689eaac7` | `5120aa1a0f53ae53bbafc58f627313d5964c50e5737dca2fcc231cc48f4ab3c8` |
| [smoke/hc-s05-osc7501-events.log](smoke/hc-s05-osc7501-events.log) | `<run-root>/evidence/smoke/hc-s05-osc7501-events.log` | 75 | RT-HC-01, RT-HC-07 | `1b771d2bd3862e99aefa8914f9bb361dd84a3c4ac11632401bf65e43ffb57d53` | `da3d2a24f5a72c36680c6472c5acf641d374488fe8ce551431a5ae099f5dad6b` |
| [smoke/hc-s06-wait-output.log](smoke/hc-s06-wait-output.log) | `<run-root>/evidence/smoke/hc-s06-wait-output.log` | 21 | RT-HC-01, RT-HC-02 | `a797fed7536668b9827f618e9aaabfd5dd8246240ecfd9cd3104d922b4555dec` | `e645b0211fc90dcfaf15e8ac8bdfe89de5364917820124ac48b0a5aecaff6afe` |
| [smoke/hc-s09-framed-attach.log](smoke/hc-s09-framed-attach.log) | `<run-root>/evidence/smoke/hc-s09-framed-attach.log` | 19 | RT-HC-05 | `4682bb95dc4d0657b48ae75c1231fb62701c465635d41178c4d686675f077a22` | `33e97c5ad3e9390aaff4f29a6f0a698ed18800407c532da13e8e14d448bb37d7` |
| [smoke/hc-s13-errors.log](smoke/hc-s13-errors.log) | `<run-root>/evidence/smoke/hc-s13-errors.log` | 21 | RT-HC-06 | `8c05bce497db66dddb789f0462854a943072af44a9e3e49dcca65fce101ebcaf` | `bb5bb28b2abedc9a74bf8eac2d8a9d85eb82a31ca5b0081a7d5ca1f9409521bd` |
| [smoke/hc-s15-label-templates.log](smoke/hc-s15-label-templates.log) | `<run-root>/evidence/smoke/hc-s15-label-templates.log` | 21 | RT-HC-03 | `40062d3c673f17c982e9472a07c6c73e566584a00b0cbac3d6e4f66adda930b9` | `74ac751158b78868adae4d2337504dadc5c60fc7651d44e29696e857fb5c36f0` |
| [smoke/hc-s18-stop-cleanup.log](smoke/hc-s18-stop-cleanup.log) | `<run-root>/evidence/smoke/hc-s18-stop-cleanup.log` | 42 | RT-HC-04 | `76ed5230a4be5f633cb6d6d24b53ed6d3cb99e72cb156fcaccf2578e58cc3381` | `1281f5bdebc4b2f3dedcf00731373b7f024dee134805b722e370995c7507a414` |
| [smoke/herdr-h02-connection.log](smoke/herdr-h02-connection.log) | `<run-root>/evidence/smoke/herdr-h02-connection.log` | 3 | RT-HD-01 | `696b0506814063032f50d059d95972b614eb8fd39742b9d3893920d007c0378b` | `696b0506814063032f50d059d95972b614eb8fd39742b9d3893920d007c0378b` |
| [smoke/herdr-h04-send-read.log](smoke/herdr-h04-send-read.log) | `<run-root>/evidence/smoke/herdr-h04-send-read.log` | 14 | RT-HD-03, RT-HD-06 | `cc92f0ac9f428106ef7319d62591a1859a3ff9696f6a9aefaa248f45a9485754` | `0b78e288701313081c3dc349df61221a908108c0573a5cc58f235c8b25ec3015` |
| [smoke/herdr-h05-process-info-trap.log](smoke/herdr-h05-process-info-trap.log) | `<run-root>/evidence/smoke/herdr-h05-process-info-trap.log` | 22 | RT-HD-02, RT-HD-03, RT-HD-04 | `b0de848f75c08d9d11b4b9f4c1824f8515ee2dcaf9a61304adc5119de861df02` | `b33f96d3a48610f9618ab0fe4a2adba3fbc2bc58e3e5975d593795b54c23be39` |
| [smoke/herdr-h06-events.log](smoke/herdr-h06-events.log) | `<run-root>/evidence/smoke/herdr-h06-events.log` | 15 | RT-HD-08 | `574df3eea633a30e069b5b0da63ac15912b215cc1f31ec1c76d1c9d73a19e21e` | `7e2fecefdd24433117dbcff53273878c22fbb0babfe7c96fc054d46e57dfb16c` |
| [smoke/herdr-h06b-single-sub.log](smoke/herdr-h06b-single-sub.log) | `<run-root>/evidence/smoke/herdr-h06b-single-sub.log` | 22 | RT-HD-08, RT-HD-09, RT-HD-10, RT-HD-11 | `4cd9e917777027be502272d60492e7d0df488eabf278b6d7832d417398836b70` | `6fcc8ccf5da5656deecfaa1a87073f1e4e0c18c8196c074d71fdc306b7ffbf31` |
| [smoke/herdr-h08-attach.log](smoke/herdr-h08-attach.log) | `<run-root>/evidence/smoke/herdr-h08-attach.log` | 6 | RT-HD-07 | `917e19db31e8fcf80a12f4f3800b6b6a0c2d19cb43a36a4ed6b07a05981be9e7` | `917e19db31e8fcf80a12f4f3800b6b6a0c2d19cb43a36a4ed6b07a05981be9e7` |
| [smoke/herdr-h10-errors.log](smoke/herdr-h10-errors.log) | `<run-root>/evidence/smoke/herdr-h10-errors.log` | 10 | RT-HD-12 | `c4e2000636d074921053c52a80f23120ac1d0e24961ab62d27537c0366bf735c` | `c4e2000636d074921053c52a80f23120ac1d0e24961ab62d27537c0366bf735c` |
| [smoke/runtime-observations.md](smoke/runtime-observations.md) | `<run-root>/evidence/smoke/runtime-observations.md` | 33 | - | `ada848d0eb0796f701de4d86f4f7511acb725768690bd49f061503f882cea48b` | `4bbda7c053218fcfb4c0ac8596957326eb418854dfd038eba8d341f90bcb49ed` |

`runtime-observations.md` is the index of RT-HC-01..08 and RT-HD-01..12 with
the transcript line each observation rests on. Plan todo 1 itself replaces no
legacy behavior: its `Before` is `absent` (GAP-145).
