#!/bin/bash
# =============================================================================
# horizon-auto-update.sh
# 100% automático: verifica atualizações do Horizon, mescla com nossas
# modificações (branch horizon-offline-mods), compila e reinicia o launcher.
#
# Estratégia:
#   - main            -> acompanha origin/main (projeto oficial)
#   - horizon-offline-mods -> nossas modificações (offline, botões 26.3, etc.)
#   - Merge: main -> horizon-offline-mods com -X ours (nossas mudanças prevalecem)
# =============================================================================
set -u

HORIZON_DIR="/mnt/jogos/horizon"
LAUNCHER_DIR="$HORIZON_DIR/launcher"
LOG_FILE="/var/log/horizon-auto-update.log"
LAUNCHER_BIN="$LAUNCHER_DIR/target/debug/horizon-launcher"
MODS_BRANCH="horizon-offline-mods"
MAIN_BRANCH="main"

log() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $1" | tee -a "$LOG_FILE"
}

cd "$HORIZON_DIR" || { log "ERRO: não consegui entrar em $HORIZON_DIR"; exit 1; }

if [ ! -d .git ]; then
    log "ERRO: $HORIZON_DIR não é um repositório git."
    exit 1
fi

log "=== Atualização automática do Horizon (100% auto) ==="

# 1. Commitar mudanças locais pendentes no branch atual
if ! git diff --quiet || ! git diff --cached --quiet; then
    log "Commitando mudanças locais pendentes..."
    git add -A
    git commit -m "auto: preserva modificações locais $(date '+%Y-%m-%d %H:%M')" >> "$LOG_FILE" 2>&1
fi

# 2. Atualizar main com o origin/main (reset hard para alinhar exatamente)
log "Atualizando $MAIN_BRANCH com o origin/$MAIN_BRANCH..."
git checkout "$MAIN_BRANCH" >> "$LOG_FILE" 2>&1
git fetch origin >> "$LOG_FILE" 2>&1
if ! git reset --hard "origin/$MAIN_BRANCH" >> "$LOG_FILE" 2>&1; then
    log "ERRO: não foi possível resetar $MAIN_BRANCH para origin/$MAIN_BRANCH."
    log "Voltando para $MODS_BRANCH..."
    git checkout "$MODS_BRANCH" >> "$LOG_FILE" 2>&1
    exit 1
fi
log "Main alinhado com o origin/$MAIN_BRANCH."

# 3. Voltar para o branch de mods e fazer merge estratégico "ours"
#    (registra que o main foi mesclado, mas mantém 100% nosso conteúdo)
log "Voltando para $MODS_BRANCH e mesclando com nossas mudanças (estratégia: ours)..."
git checkout "$MODS_BRANCH" >> "$LOG_FILE" 2>&1

if ! git merge -s ours "$MAIN_BRANCH" >> "$LOG_FILE" 2>&1; then
    log "ERRO: não foi possível fazer merge estratégico ours."
    log "Voltando para estado anterior..."
    git reset --hard HEAD@{1} >> "$LOG_FILE" 2>&1 || true
    exit 1
fi
log "Merge concluído (conteúdo 100% nosso, main registrado como mesclado)."

# 4. Compilar
if [ -f "$LAUNCHER_DIR/Cargo.toml" ]; then
    log "Verificando compilação do launcher..."
    cd "$LAUNCHER_DIR"
    if cargo check >> "$LOG_FILE" 2>&1; then
        log "Compilação OK."
    else
        log "ERRO: compilação falhou após merge. Desfazendo merge..."
        cd "$HORIZON_DIR"
        git merge --abort >> "$LOG_FILE" 2>&1 || true
        git reset --hard HEAD@{1} >> "$LOG_FILE" 2>&1 || true
        exit 1
    fi
fi

# 5. Build release (opcional, deixa pronto para usar)
# log "Fazendo build release..."
# cd "$LAUNCHER_DIR" && cargo build --release >> "$LOG_FILE" 2>&1

# 6. Reiniciar o launcher se estiver rodando
if pgrep -f "horizon-launcher" > /dev/null 2>&1; then
    log "Reiniciando o launcher..."
    pkill -f "horizon-launcher" >> "$LOG_FILE" 2>&1
    sleep 2
    nohup "$LAUNCHER_BIN" >> /var/log/horizon-launcher.log 2>&1 &
    log "Launcher reiniciado."
fi

log "=== Atualização concluída com sucesso ==="
exit 0
