# Trechos comuns aos scripts que geram os pacotes da Jangada nas outras distribuições
# (ci/fedora.sh, ci/arch.sh, ci/slackware.sh). Para ser lido com ". ci/comum.sh".

# O Rust: o do sistema, se já houver um novo o bastante (o Slackware -current traz o dele
# na série d); senão, o do rustup. Com o do sistema não existe ~/.cargo/env: ler esse
# arquivo sem conferir derrubava o pacote do Slackware depois de horas compilando o motor.
instalar_rust() {
    minimo=$(sed -n 's/^rust-version = "1\.\([0-9]*\).*/\1/p' Cargo.toml | head -1)
    atual=$(rustc --version 2>/dev/null | sed -n 's/^rustc 1\.\([0-9]*\)\..*/\1/p')
    if command -v cargo >/dev/null 2>&1 && [ -n "$atual" ] && [ "$atual" -ge "${minimo:-0}" ]; then
        echo "==> Rust do sistema: $(rustc --version)"
        return 0
    fi
    # (IPv4: no contêiner do Slackware o IPv6 não sai.)
    if command -v curl >/dev/null 2>&1 && curl -V >/dev/null 2>&1; then
        curl -4 --proto '=https' --tlsv1.2 -sSf --retry 3 https://sh.rustup.rs | sh -s -- -y --profile minimal
    else
        wget -4 -q -O /tmp/rustup-init "https://static.rust-lang.org/rustup/dist/$(uname -m)-unknown-linux-gnu/rustup-init"
        chmod +x /tmp/rustup-init
        /tmp/rustup-init -y --profile minimal
    fi
    . "$HOME/.cargo/env"
}

# Depois de instalar o pacote gerado: os arquivos estão no lugar e o programa acha todas
# as bibliotecas (um pacote com dependência errada falha aqui, e não no computador de quem
# baixou).
conferir() {
    for f in /usr/bin/jangada /usr/lib/jangada/jangada /usr/share/applications/jangada.desktop \
             /usr/share/icons/hicolor/48x48/apps/jangada.png; do
        [ -e "$f" ] || { echo "ERRO: o pacote não instalou $f"; exit 1; }
    done
    falta=$(ldd /usr/lib/jangada/jangada | grep "not found" || true)
    [ -z "$falta" ] || { echo "ERRO: a Jangada não acha:"; echo "$falta"; exit 1; }
    # O programa abre (sem tela: só gera os ícones e sai).
    d=$(mktemp -d)
    /usr/lib/jangada/jangada --icons "$d" || { echo "ERRO: a Jangada não abre"; exit 1; }
    rm -rf "$d"
    echo "Pacote conferido: instalado, com todas as bibliotecas."
}
