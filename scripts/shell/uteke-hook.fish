# Uteke shell hook for Fish
# Add to ~/.config/fish/config.fish: uteke hook fish | source
function __uteke_find_project
    set -l dir $PWD
    while test "$dir" != "/"
        if test -f "$dir/.uteke/uteke.db"
            echo "$dir/.uteke"
            return 0
        end
        set dir (dirname "$dir")
    end
    return 1
end

function __uteke_directory_change --on-variable PWD
    set -l project_store (__uteke_find_project)
    if test -n "$project_store"
        set -gx UTEKE_PROJECT_STORE "$project_store"
    else
        set -e UTEKE_PROJECT_STORE
    end
end
