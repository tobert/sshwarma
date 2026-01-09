-- sshwarma:create - Create a new room
return function(args)
    local name = args and args.name or args
    if not name then
        return {success = false, error = "room name required"}
    end
    return tools.create(name)
end
