# Uteke shell hook for Zsh
# Add to ~/.zshrc: eval "$(uteke hook zsh)"
chpwd_functions+=(uteke_chpwd)
uteke_chpwd() {
    if [[ -f ".uteke/uteke.db" ]]; then
        export UTEKE_PROJECT_STORE="$PWD/.uteke"
    else
        unset UTEKE_PROJECT_STORE
    fi
}
