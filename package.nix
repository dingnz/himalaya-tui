{
  buildFeatures ? [ ],
  buildNoDefaultFeatures ? false,
  buildPackages,
  fetchFromGitHub,
  installManPages ? stdenv.buildPlatform.canExecute stdenv.hostPlatform,
  installShellCompletions ? stdenv.buildPlatform.canExecute stdenv.hostPlatform,
  installShellFiles,
  lib,
  pkg-config,
  rustPlatform,
  stdenv,
}:

let
  emulator = stdenv.hostPlatform.emulator buildPackages;
  exe = stdenv.hostPlatform.extensions.executable;
  withNativeTls = builtins.elem "native-tls" buildFeatures;

in
rustPlatform.buildRustPackage {
  inherit buildNoDefaultFeatures;

  pname = "himalaya-tui";
  version = "0.0.1";
  cargoHash = "";

  src = fetchFromGitHub {
    hash = "";
    owner = "pimalaya";
    repo = "himalaya-tui";
    rev = "v0.0.1";
  };

  env = {
    # OpenSSL should not be provided by vendors, not even on Windows
    OPENSSL_NO_VENDOR = "1";
  };

  nativeBuildInputs = [
    pkg-config
    installShellFiles
  ];

  buildFeatures =
    buildFeatures ++ lib.optional (withNativeTls && stdenv.hostPlatform.isWindows) "vendored";

  # most of the tests are lib side
  doCheck = false;

  postInstall =
    lib.optionalString (lib.hasInfix "wine" emulator) ''
      export WINEPREFIX="''${WINEPREFIX:-$(mktemp -d)}"
      mkdir -p $WINEPREFIX
    ''
    + ''
      mkdir -p $out/share/{applications,completions,man}
      ${emulator} "$out"/bin/himalaya-tui${exe} man "$out"/share/man
      ${emulator} "$out"/bin/himalaya-tui${exe} completion -d "$out"/share/completions bash elvish fish powershell zsh
    ''
    + lib.optionalString installManPages ''
      installManPage "$out"/share/man/*
    ''
    + lib.optionalString installShellCompletions ''
      installShellCompletion --cmd himalaya-tui \
        --bash "$out"/share/completions/himalaya-tui.bash \
        --fish "$out"/share/completions/himalaya-tui.fish \
        --zsh "$out"/share/completions/_himalaya-tui
    '';

  meta = {
    description = "TUI to manage emails";
    mainProgram = "himalaya-tui";
    homepage = "https://github.com/pimalaya/himalaya-tui";
    changelog = "https://github.com/pimalaya/himalaya-tui/blob/v0.0.1/CHANGELOG.md";
    license = lib.licenses.agpl3Only;
    maintainers = with lib.maintainers; [ soywod ];
  };
}
