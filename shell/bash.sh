# Include this file in your ~/.bashrc to get a convenient `code` function for
# switching between repos, with support for Tab completion. You can also switch
# back and forth between two codebases with `code -`.

# Replace with the directory where your git repos reside.
__codepath="$HOME/Code"

__previous_code=
__current_code=
code() {
    local dir
    if [ $# -eq 1 ] && [[ $1 == - ]]; then
        if [ -z "${__previous_code}" ]; then
            echo "No previous codebase!" >&2
            return 1
        fi
        dir="${__previous_code}"
    elif dir=$(codeswitch "${__codepath}" "$@"); then
        __previous_code=$__current_code
        __current_code=$dir
    else
        echo "${dir}"
        return 1
    fi
    cd "${dir}" || return 1
}

__code_complete() {
    local cur opts
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"

    # name of the repo
    if [ "$COMP_CWORD" = 1 ]; then
        opts=$(codeswitch "${__codepath}" '_')
    else
        return 0
    fi

    COMPREPLY=( $(compgen -W "${opts}" -- ${cur}) )
}

complete -F __code_complete code
