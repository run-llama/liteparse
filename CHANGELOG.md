# @llamaindex/liteparse

## 1.4.6

### Patch Changes

- [#120](https://github.com/run-llama/liteparse/pull/120) [`9cde441`](https://github.com/run-llama/liteparse/commit/9cde441b106b9dbee055b228bf011c197126bef5) Thanks [@apprakash](https://github.com/apprakash)! - fix(gridProjection): merge sub-pixel overlapping text runs in canMerge

## 1.4.5

### Patch Changes

- [#118](https://github.com/run-llama/liteparse/pull/118) [`4fdf3c9`](https://github.com/run-llama/liteparse/commit/4fdf3c9358c22e50e742c2b7d866ae35b83a63cd) Thanks [@mkh09353](https://github.com/mkh09353)! - Honor the OCR rotation-correction option in the built-in Tesseract engine by mapping it to Tesseract.js auto-rotation.

## 1.4.4

### Patch Changes

- [#112](https://github.com/run-llama/liteparse/pull/112) [`0eda8fc`](https://github.com/run-llama/liteparse/commit/0eda8fc27d6ad2cf835894efecc22c5239025447) Thanks [@logan-markewich](https://github.com/logan-markewich)! - Improve buggy-font handling resolution

## 1.4.3

### Patch Changes

- [#108](https://github.com/run-llama/liteparse/pull/108) [`f4ee121`](https://github.com/run-llama/liteparse/commit/f4ee121d800b01f007a1d970d8197eda6673b392) Thanks [@Winds-AI](https://github.com/Winds-AI)! - Fix OCR bullet line spacing inflation

## 1.4.2

### Patch Changes

- [#91](https://github.com/run-llama/liteparse/pull/91) [`5bb3a3b`](https://github.com/run-llama/liteparse/commit/5bb3a3b214b148bec86aaf979ea561611a7df763) Thanks [@AdemBoukhris457](https://github.com/AdemBoukhris457)! - fix: use path.join for screenshot output filepath

- [#97](https://github.com/run-llama/liteparse/pull/97) [`1100bdb`](https://github.com/run-llama/liteparse/commit/1100bdbcb7293abb63d3eb38ff295669618265e0) Thanks [@AdemBoukhris457](https://github.com/AdemBoukhris457)! - fix: return null from extension detection for unrecognizable formats

- [#89](https://github.com/run-llama/liteparse/pull/89) [`71f6621`](https://github.com/run-llama/liteparse/commit/71f6621dd413195b3634b747c6cf7cde90966035) Thanks [@AdemBoukhris457](https://github.com/AdemBoukhris457)! - perf: cache PDFium document across page operations

- [#99](https://github.com/run-llama/liteparse/pull/99) [`b7a3080`](https://github.com/run-llama/liteparse/commit/b7a3080d89dccc4fd3cec65f559d4ebeff12e9bc) Thanks [@Winds-AI](https://github.com/Winds-AI)! - fix: validate ImageMagick executables before using convert

- [#95](https://github.com/run-llama/liteparse/pull/95) [`2718912`](https://github.com/run-llama/liteparse/commit/2718912b520ffc475d8e2541f5430b0823bd0acd) Thanks [@AdemBoukhris457](https://github.com/AdemBoukhris457)! - fix: guard indexOf before splice in grid anchor resolution

## 1.4.1

### Patch Changes

- [#84](https://github.com/run-llama/liteparse/pull/84) [`53e02df`](https://github.com/run-llama/liteparse/commit/53e02dff71d8f83cb2539b3a856889a5c3a38b52) Thanks [@AdemBoukhris457](https://github.com/AdemBoukhris457)! - Ensure parse cleans up temp files

- [#86](https://github.com/run-llama/liteparse/pull/86) [`48d86f1`](https://github.com/run-llama/liteparse/commit/48d86f1f2a0fc9d0cad29d3d30476f5cd0844d85) Thanks [@AdemBoukhris457](https://github.com/AdemBoukhris457)! - Ensure screenshot converts formats when possible

## 1.4.0

### Minor Changes

- [#64](https://github.com/run-llama/liteparse/pull/64) [`ab3df58`](https://github.com/run-llama/liteparse/commit/ab3df583fcbf6f0333a0649f7b4bd7331e5d547a) Thanks [@llrightll](https://github.com/llrightll)! - Add confidence scores to TextItems

- [#71](https://github.com/run-llama/liteparse/pull/71) [`57adda1`](https://github.com/run-llama/liteparse/commit/57adda15e6a45832e7f3a1311fb475c7221c1dc8) Thanks [@saravananravi08](https://github.com/saravananravi08)! - Add internal image detection for OCR

### Patch Changes

- [#78](https://github.com/run-llama/liteparse/pull/78) [`d341371`](https://github.com/run-llama/liteparse/commit/d341371eae7c2fa8feb234af732cf30e978230b3) Thanks [@logan-markewich](https://github.com/logan-markewich)! - Improve searchItems output on complex text

## 1.3.2

### Patch Changes

- [#55](https://github.com/run-llama/liteparse/pull/55) [`b57cb61`](https://github.com/run-llama/liteparse/commit/b57cb61de9371cbc1cf91f01aafc7e1fe912e520) Thanks [@hexapode](https://github.com/hexapode)! - Improve text projection on justified text

## 1.3.1

### Patch Changes

- [#70](https://github.com/run-llama/liteparse/pull/70) [`243dc05`](https://github.com/run-llama/liteparse/commit/243dc0556769a59cf59e6565a5657b7d2630fc97) Thanks [@saravananravi08](https://github.com/saravananravi08)! - fix: resolve standard font loading failure in Node.js

## 1.3.0

### Minor Changes

- [#67](https://github.com/run-llama/liteparse/pull/67) [`0542758`](https://github.com/run-llama/liteparse/commit/0542758f6239a1897d7553727ce3ec58c61ea7fe) Thanks [@logan-markewich](https://github.com/logan-markewich)! - Bbox utils and tesseract error handling

## 1.2.0

### Minor Changes

- [#56](https://github.com/run-llama/liteparse/pull/56) [`31b43f9`](https://github.com/run-llama/liteparse/commit/31b43f9666ce6df85e90a44be1e859c615bda757) Thanks [@logan-markewich](https://github.com/logan-markewich)! - Add CLI Stdin support for files, urls, etc.

## 1.1.0

### Minor Changes

- [#51](https://github.com/run-llama/liteparse/pull/51) [`7b421c6`](https://github.com/run-llama/liteparse/commit/7b421c61f2e2ffa04e68bb2bbe02dbf18e261507) Thanks [@logan-markewich](https://github.com/logan-markewich)! - Support for password protected PDFs

## 1.0.1

### Patch Changes

- [#40](https://github.com/run-llama/liteparse/pull/40) [`bb863c4`](https://github.com/run-llama/liteparse/commit/bb863c46f568c5c192e7c6ec608e350303668bba) Thanks [@logan-markewich](https://github.com/logan-markewich)! - Add support for TESSDATA_PREFIX and better error messaging on tesseract network errors

## 1.0.0

### Major Changes

- [#31](https://github.com/run-llama/liteparse/pull/31) [`56ba21c`](https://github.com/run-llama/liteparse/commit/56ba21cb63e8223440b039f49eab710ba089e375) Thanks [@logan-markewich](https://github.com/logan-markewich)! - LiteParse v1.0 launch
