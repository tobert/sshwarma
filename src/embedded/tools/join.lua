-- sshwarma:join - Join a room
return function(args)
    local room = args and args.room or args
    if not room then
        return {success = false, error = "room name required"}
    end
    return tools.join(room)
end
