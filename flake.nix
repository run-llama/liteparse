{
  description = "LiteParse: fast, lightweight PDF and document parsing with spatial text extraction";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # Keep in sync with PDFIUM_RELEASE_TAG in upstream:
        # crates/pdfium-sys/build.rs
        pdfiumAsset =
          {
            "x86_64-linux" = "pdfium-linux-x64";
            "aarch64-linux" = "pdfium-linux-arm64";
          }.${system};

        pdfiumHash =
          {
            "pdfium-linux-x64" = "sha256-LdQYqJdNuzc60oH5m2+V7QLgGxorsuBedyyAt1h1d7U=";
            "pdfium-linux-arm64" = "sha256-SRW1NRZAwe6lniYCPg7crYQz35bcj0yz1kWk1koXi3w=";
          }.${pdfiumAsset};

        pdfium = pkgs.stdenv.mkDerivation {
          pname = "pdfium";
          version = "chromium-8058";
          __structuredAttrs = true;
          strictDeps = true;

          src = pkgs.fetchurl {
            url = "https://github.com/run-llama/pdfium-binaries/releases/download/chromium%2F8058/${pdfiumAsset}.tgz";
            hash = pdfiumHash;
          };

          unpackPhase = ''
            mkdir source
            tar -xzf $src -C source
          '';
          sourceRoot = "source";

          installPhase = ''
            cp -r . $out
          '';
        };

        mkLiteparse =
          pkgs.rustPlatform.buildRustPackage (finalAttrs: {
            pname = "liteparse";
            version = "2.18.0";
            __structuredAttrs = true;

            src = self;

            postPatch = ''
              mkdir -p .tesseract-rs/third_party
              ln -s ${pkgs.leptonica.src} .tesseract-rs/third_party/leptonica
              ln -s ${pkgs.tesseract.tesseractBase.src} .tesseract-rs/third_party/tesseract

              # pdfium-sys tries to copy libpdfium into the (read-only in Nix) deps dir;
              # skip that best-effort packaging step, runtime uses PDFIUM_LIB_PATH.
              substituteInPlace crates/pdfium-sys/build.rs \
                --replace-fail 'if deps_dir.is_dir() {' 'if false && deps_dir.is_dir() {'


            ''
            + ''
              tesseractRsBuildRs=$(find "$cargoDepsCopy" -path '*/tesseract-rs-*/build.rs' | head -n1)
              if [ -n "$tesseractRsBuildRs" ]; then
                substituteInPlace "$tesseractRsBuildRs" \
                  --replace-fail \
                    '.define("CMAKE_INSTALL_PREFIX", &leptonica_install_dir)' \
                    '.define("CMAKE_INSTALL_PREFIX", &leptonica_install_dir)
                                .define("CMAKE_INSTALL_LIBDIR", "lib")' \
                  --replace-fail \
                    '.define("CMAKE_INSTALL_PREFIX", &tesseract_install_dir)' \
                    '.define("CMAKE_INSTALL_PREFIX", &tesseract_install_dir)
                                .define("CMAKE_INSTALL_LIBDIR", "lib")'
              fi

              zstdSysBuildRs=$(find "$cargoDepsCopy" -path '*/zstd-sys-*/build.rs' | head -n1)
              if [ -n "$zstdSysBuildRs" ]; then
                substituteInPlace "$zstdSysBuildRs" \
                  --replace-fail 'cfg!(feature = "pkg-config")' 'false' \
                  --replace-fail 'env::var_os("ZSTD_SYS_USE_PKG_CONFIG").is_some()' 'false'
              fi
            '';

            nativeBuildInputs = with pkgs; [
              cmake
              makeWrapper
              pkg-config
              writableTmpDirAsHomeHook
              which
              libreoffice
            ];

            buildInputs = with pkgs; [
              openssl
              aws-lc
              leptonica
              cacert
              libreoffice
              which
            ];

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            cargoBuildFlags = [
              "--package"
              "liteparse"
            ];
            cargoTestFlags = [
              "--package"
              "liteparse"
              "--"
              "--skip"
              "test_batch_session_owns_converted_source_for_its_lifetime"
              "--skip"
              "test_parse_office_doc_integration"
              "--skip"
              "test_parse_bytes_office_integration"
            ];
            cargoInstallFlags = [
              "--path"
              "crates/liteparse"
            ];
            doCheck = true;

            preBuild = ''
              mkdir -p "$HOME/.tesseract-rs"
              cp -rL .tesseract-rs/third_party "$HOME/.tesseract-rs/"
              chmod -R u+w "$HOME/.tesseract-rs"
              mkdir -p "$HOME/.tesseract-rs/tessdata"
              cp ${pkgs.tesseract.languages.eng} "$HOME/.tesseract-rs/tessdata/eng.traineddata"
              cp ${pkgs.tesseract.languages.tur} "$HOME/.tesseract-rs/tessdata/tur.traineddata"
            '';

            env = {
              OPENSSL_NO_VENDOR = true;
              AWS_LC_SYS_USE_SYSTEM = "1";
              PDFIUM_INCLUDE_PATH = "${pdfium}/include";
              PDFIUM_LIB_PATH = "${pdfium}/lib";
              SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
              NIX_SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
            };

            postInstall = ''
              wrapProgram $out/bin/lit \
                --set PDFIUM_LIB_PATH "${pdfium}/lib" \
                --set TESSDATA_PREFIX "${pkgs.tesseract}/share/tessdata" \
                --prefix PATH : "${pkgs.libreoffice}/bin"
            '';

            nativeInstallCheckInputs = [
              pkgs.versionCheckHook
            ];
            doInstallCheck = true;

            meta = {
              description = "Fast and lightweight document parser by LlamaIndex";
              homepage = "https://github.com/run-llama/liteparse";
              license = pkgs.lib.licenses.asl20;
              mainProgram = "lit";
              platforms = with pkgs.lib.platforms; linux;
            };
          });
      in
      {
        packages.default = mkLiteparse;
        packages.liteparse = mkLiteparse;

        apps.default = {
          type = "app";
          program = "${mkLiteparse}/bin/lit";
          meta = mkLiteparse.meta;
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ mkLiteparse ];
          packages = [ pkgs.cargo pkgs.rustc ];
        };
      }
    );
}