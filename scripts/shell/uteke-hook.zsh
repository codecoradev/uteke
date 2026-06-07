# Uteke shell hook for Zsh
# Add to ~/.zshrc: eval "$(uteke hook zsh)"
_uteke_find_project() {
    local dir="$PWD"
    while [[ "$dir" != "/" ]]; do
        if [[ -f "$dir/.uteke/uteke.db" ]]; then
            echo "$dir/.uteke"
            return 0
        fi
        dir="$(dirname "$dir")"
    done
    return 1
}

uteke_chpwd() {
    local project_store
    if project_store="$(_uteke_find_project)"; then
        export UTEKE_PROJECT_STORE="$project_store"
    else
        unset UTEKE_PROJECT_STORE
    fi
}
# Guard against duplicate sourcing
if [[ " ${chpwd_functions[@]} " != *" uteke_chpwd "* ]]; then
    chpwd_functions+=(uteke_chpwd)
fi
