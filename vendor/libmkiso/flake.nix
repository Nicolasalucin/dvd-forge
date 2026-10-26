{
  description = "Rust and Windows cross-compilation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
    in
    {
      formatter = forAllSystems (system: (pkgsFor system).nixfmt);
      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          rust = pkgs.rust-bin.stable."1.96.0".default.override {
            extensions = [ "rust-src" ];
            targets = [
              "x86_64-pc-windows-msvc"
              "i686-pc-windows-msvc"
              "wasm32-unknown-unknown"
            ];
          };
        in
        {
          default = pkgs.mkShell {
            packages = [
              rust
              pkgs.go-task
              pkgs.python3
              pkgs.sccache
              pkgs.nixfmt
              pkgs.cargo-xwin
              pkgs.clang
              pkgs.lld
              pkgs.llvm
              pkgs.actionlint
              pkgs.powershell
              pkgs.shellcheck
              # Native dependencies for honggfuzz's instrumented builds.
              pkgs.gnumake
              pkgs.binutils-unwrapped
              pkgs.libunwind
              pkgs.xz
            ];
            XWIN_ARCH = "x86,x86_64";
            shellHook = "";
          };
        }
      );
    };
}
