# Uteke shell hook for Fish
# Add to ~/.config/fish/config.fish: uteke hook fish | source
function __uteke_directory_change --on-variable PWD
    if test -f .uteke/uteke.db
        set -gx UTEKE_PROJECT_STORE "$PWD/.uteke"
    else
        set -e UTEKE_PROJECT_STORE
    end
end
