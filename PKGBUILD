pkgname=plus-a
pkgver=1.0.0
pkgrel=1
pkgdesc="ASCENSION +A assembly language toolchain"
arch=('x86_64')
url="https://github.com/ardis-star/A"
license=('custom')
makedepends=('rust')
source=("$pkgname-$pkgver.tar.gz::https://github.com/ardis-star/A/archive/refs/tags/V1.0.0.tar.gz")
sha256sums=('5a2780f62d4454c401ee4a449913d749e7e49dc06cf45b5bcf50f2f53f10795e')

build() {
    cd "$srcdir/A-1.0.0/rust"
    cargo build --release --locked
}

package() {
    cd "$srcdir/A-1.0.0"

    install -Dm755 rust/target/release/plus-a "$pkgdir/usr/bin/plus-a"
    install -Dm755 rust/target/release/aa "$pkgdir/usr/bin/aa"

    ln -s plus-a "$pkgdir/usr/bin/+a"

    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
