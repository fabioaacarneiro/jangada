# Trechos comuns aos scripts que geram os pacotes da Jangada nas outras distribuições
# (ci/fedora.sh, ci/arch.sh, ci/slackware.sh). Para ser lido com ". ci/comum.sh".

# O Rust pelo rustup (a mesma versão em todas as distribuições).
instalar_rust() {
    if ! command -v cargo >/dev/null 2>&1; then
        if command -v curl >/dev/null 2>&1 && curl -V >/dev/null 2>&1; then
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
        else
            wget -q -O /tmp/rustup-init "https://static.rust-lang.org/rustup/dist/$(uname -m)-unknown-linux-gnu/rustup-init"
            chmod +x /tmp/rustup-init
            /tmp/rustup-init -y --profile minimal
        fi
    fi
    . "$HOME/.cargo/env"
}

# Depois de instalar o pacote gerado: os arquivos estão no lugar e o programa acha todas
# as bibliotecas (um pacote com dependência errada falha aqui, e não no computador de quem
# baixou).
conferir() {
    for f in /usr/bin/jangada /usr/lib/jangada/jangada /usr/lib/jangada/libcef.so \
             /usr/lib/jangada/chrome-sandbox /usr/share/applications/jangada.desktop; do
        [ -e "$f" ] || { echo "ERRO: o pacote não instalou $f"; exit 1; }
    done
    falta=$(ldd /usr/lib/jangada/jangada | grep "not found" || true)
    [ -z "$falta" ] || { echo "ERRO: a Jangada não acha:"; echo "$falta"; exit 1; }
    falta=$(ldd /usr/lib/jangada/libcef.so | grep "not found" || true)
    [ -z "$falta" ] || { echo "ERRO: o motor não acha:"; echo "$falta"; exit 1; }
    ls -l /usr/lib/jangada/chrome-sandbox
    echo "Pacote conferido: instalado, com todas as bibliotecas."
}
