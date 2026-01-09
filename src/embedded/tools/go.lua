-- sshwarma:go - Navigate through an exit
return function(args)
    local direction = args and args.direction or args
    if not direction then
        return {success = false, error = "direction required"}
    end
    return tools.go(direction)
end
