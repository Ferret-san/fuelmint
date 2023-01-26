#!/bin/sh
set -e

FUELMINT_DIR=${FUELMINT_DIR-"$HOME/.fuelmint"}

ROLLKIT_NODE=${ROLLKIT_NODE-"$HOME/.rollkit-node"}

main() {

    mkdir -p "$FUELMINT_DIR/bin"

    mkdir -p "$ROLLKIT_NODE/bin"

    case $SHELL in
            */bash)
                SHELL_PROFILE=$HOME/.bashrc
                ;;
            */zsh)
                SHELL_PROFILE=$HOME/.zshrc
                ;;
            */fish)
                SHELL_PROFILE=$HOME/.config/fish/config.fish
                ;;
            *)
                warn "Failed to detect shell; please add ${FUELUP_DIR}/bin to your PATH manually."
                ;;
        esac


    cargo build --release

    ensure mv -f  "target/release/fuelmint" "$FUELMINT_DIR/bin"
    
    echo "export PATH=\"\$HOME/.fuelmint/bin:\$PATH"\" >>"$SHELL_PROFILE"

    cd rollkit-node

    go build
    
    ensure mv -f -n "./rollkit-node" "$ROLLKIT_NODE/bin"

    echo "export PATH=\"\$HOME/.rollkit-node/bin:\$PATH"\" >>"$SHELL_PROFILE"

    cd ..
}

# Run a command that should never fail. If the command fails execution
# will immediately terminate with an error showing the failing
# command.
ensure() {
    if ! "$@"; then err "command failed: $*"; fi
}

main "$@" || exit 1
