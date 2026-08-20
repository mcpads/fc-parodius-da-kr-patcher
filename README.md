# 파로디우스다! (패미컴) 한글 패처

패미컴용 《파로디우스다!》 일본판에서 한글판 ROM과 BPS를 만드는 Rust 구현입니다. 지원 원본 확인, NES 2.0 PRG·CHR 확장, 글꼴·그래픽 변환, 엔딩 재배치, 변경 범위 추적, BPS 생성·적용 검증 코드를 포함합니다.

배포용 BPS와 적용 방법은 [파로디우스 시리즈 한글 번역 프로젝트](https://github.com/mcpads/parodius-kr-patch)에서 제공합니다.

## 공개 범위

이 저장소에는 원본 ROM, 완성 ROM, 배포용 BPS, 번역 원고와 검수 자료, 제품용 글꼴·타이틀 그래픽이 없습니다. 공개된 코드는 원본과 외부 자산을 검사하고 결합하는 구현이며, 현재 공개 트리만으로 배포 BPS를 다시 만들 수는 없습니다.

제품 빌드에는 다음 입력이 별도로 필요합니다.

- 지원 일본판 ROM
- 한글 타이틀 RGBA PNG
- 승인된 그래픽 번역 데이터
- 승인된 엔딩 번역 데이터
- `assets/fonts/dalmoori.ttf`와 `assets/fonts/NEXONLv2Gothic.ttf`

원본과 외부 자산의 취득·이용·재배포 권리는 사용자가 확인해야 하며, 이 저장소의 MIT 라이선스가 그 파일들에 적용되지는 않습니다.

## 지원 원본

통합 빌더가 받는 원본은 iNES 헤더를 포함한 262,160바이트 일본판 ROM이며 SHA-1은 다음과 같습니다.

```text
fc8ee2c4d869b1d09ad4d66d3c2d557336a6560d
```

배포 프로젝트는 헤더 없는 PRG+CHR 원본용 BPS를 기본으로 제공하고, iNES 헤더 포함 원본용 BPS를 호환용으로 제공합니다. 두 패치는 하나의 통합 빌드 결과에서 생성됩니다.

## 빌드와 테스트

외부 자산이 없는 공개 체크아웃에서도 서식은 확인할 수 있습니다.

```bash
cargo fmt --all -- --check
```

컴파일과 테스트에는 위의 두 글꼴이 필요하고, 전체 테스트에는 `assets/title/korean-title-genai.png`도 필요합니다. 해당 파일을 준비한 뒤 다음 검사를 실행합니다.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

최종 제품 빌드는 `build-korean-draft` 명령에 원본 ROM과 타이틀·그래픽 번역·엔딩 번역 입력을 명시적으로 전달합니다. 입력 형식과 옵션은 다음 명령에서 확인할 수 있습니다.

```bash
cargo run -p nes-parodius-patch -- build-korean-draft --help
```

## 라이선스

이 저장소의 소스 코드는 [MIT License](LICENSE)로 제공합니다.
