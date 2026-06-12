# Uteke shell hook for Bash
# Add to ~/.bashrc: eval "$(uteke hook bash)"
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

_uteke_prev_dir="$PWD"
uteke_prompt_hook() {
    if [[ "$PWD" != "$_uteke_prev_dir" ]]; then
        _uteke_prev_dir="$PWD"
        local project_store
        if project_store="$(_uteke_find_project)"; then
            export UTEKE_PROJECT_STORE="$project_store"
        else
            unset UTEKE_PROJECT_STORE
        fi
    fi
}
# Guard against duplicate sourcing
if [[ ":${PROMPT_COMMAND}:" != *":uteke_prompt_hook:"* ]]; then
    PROMPT_COMMAND="uteke_prompt_hook;${PROMPT_COMMAND}"
fi
