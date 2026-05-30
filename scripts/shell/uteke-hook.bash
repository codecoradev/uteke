# Uteke shell hook for Bash
# Add to ~/.bashrc: eval "$(uteke hook bash)"
_uteke_prev_dir="$PWD"
uteke_prompt_hook() {
    if [[ "$PWD" != "$_uteke_prev_dir" ]]; then
        _uteke_prev_dir="$PWD"
        if [[ -f ".uteke/uteke.db" ]]; then
            export UTEKE_PROJECT_STORE="$PWD/.uteke"
        else
            unset UTEKE_PROJECT_STORE
        fi
    fi
}
PROMPT_COMMAND="uteke_prompt_hook;${PROMPT_COMMAND}"
