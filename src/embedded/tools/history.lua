-- sshwarma:history - View conversation history
return function(args)
    local limit = args and args.limit or 20
    return tools.history(limit)
end
