(use_decl
	address: (address_literal) @address
    module: (module_identifier) @module_name
    as: (module_identifier)? @module_alias
    (use_member
        member: (identifier) @member
        as: (module_identifier)? @member_alias)?)