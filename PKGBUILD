# Maintainer: Yakup Atahanov <yakupatahanov@users.noreply.github.com>

pkgname=dispatch-mcp-git
pkgver=0.1.0.r0.g0000000
pkgrel=1
pkgdesc='Signal-driven task orchestrator for MCP servers'
arch=('x86_64' 'aarch64')
url='https://github.com/YakupAtahanov/dispatch'
license=('GPL-3.0-or-later')
depends=('gcc-libs')
makedepends=('cargo' 'git')
optdepends=('dmcp: MCP server manager for discovering and invoking servers')
provides=('dispatch-mcp' 'dispatch')
conflicts=('dispatch-mcp')
source=("${pkgname}::git+${url}.git")
sha256sums=('SKIP')

pkgver() {
  cd "${pkgname}"
  git describe --long --tags --abbrev=7 2>/dev/null | sed 's/^v//;s/-/.r/;s/-/./' \
    || printf '0.1.0.r%s.g%s' "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"
}

prepare() {
  cd "${pkgname}"
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "${pkgname}"
  export RUSTUP_TOOLCHAIN=stable
  export CARGO_TARGET_DIR=target
  cargo build --frozen --release
}

package() {
  cd "${pkgname}"
  install -Dm755 "target/release/dispatch" "${pkgdir}/usr/bin/dispatch"
  install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
}
