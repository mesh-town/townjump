include $(TOPDIR)/rules.mk

PKG_NAME:=townjump
PKG_VERSION:=0.1.0
PKG_RELEASE:=1

PKG_SOURCE:=$(PKG_NAME)-$(PKG_VERSION).tar.gz
PKG_SOURCE_URL:=https://codeload.github.com/mesh-town/townjump/tar.gz/v$(PKG_VERSION)?
PKG_BUILD_DIR:=$(BUILD_DIR)/townjump-$(PKG_VERSION)
# Fill in after the release: make package/townjump/download prints the
# expected hash (or: shasum -a 256 <tarball>).
PKG_HASH:=skip

PKG_MAINTAINER:=Mesh Town <meshtown.app@gmail.com>
PKG_LICENSE:=AGPL-3.0-only
PKG_LICENSE_FILES:=LICENSE

PKG_BUILD_DEPENDS:=rust/host
PKG_BUILD_PARALLEL:=1

# The binary crate lives in a subdirectory of the workspace.
MAKE_PATH:=crates/townjump

include ../../lang/rust/rust-package.mk

define Package/townjump
  SECTION:=net
  CATEGORY:=Network
  TITLE:=Unofficial Rust port of Holesail (P2P TCP/UDP tunnels)
  URL:=https://github.com/mesh-town/townjump
  # 32-bit MIPS has no AtomicU64, so mips/mipsel/mips64 are left out.
  DEPENDS:=@(aarch64||arm||i386||loongarch64||powerpc||powerpc64||riscv64||x86_64)
endef

define Package/townjump/description
  Unofficial Rust port of Holesail, not affiliated with Holesail / LIVEPORT.
  Installs the `townjump` command, so it does not collide with the npm
  package `holesail`; its help and messages still say `holesail`.
endef

$(eval $(call RustBinPackage,townjump))
$(eval $(call BuildPackage,townjump))
