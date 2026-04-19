pub(super) fn tool_argument_hint(name: &str) -> &'static str {
    if let Some(argument_hint) = crate::tools::tool_surface::direct_tool_argument_hint(name) {
        return argument_hint;
    }

    match name {
        "feishu.bitable.app.create" => {
            "account_id?:string,open_id?:string,name:string,folder_token?:string"
        }
        "feishu.bitable.app.get" => "account_id?:string,open_id?:string,app_token:string",
        "feishu.bitable.app.list" => {
            "account_id?:string,open_id?:string,folder_token?:string,page_size?:integer,page_token?:string"
        }
        "feishu.bitable.app.patch" => {
            "account_id?:string,open_id?:string,app_token:string,name?:string,is_advanced?:boolean"
        }
        "feishu.bitable.app.copy" => {
            "account_id?:string,open_id?:string,app_token:string,name:string,folder_token?:string"
        }
        "feishu.bitable.list" => {
            "account_id?:string,open_id?:string,app_token:string,page_size?:integer,page_token?:string"
        }
        "feishu.bitable.table.create" => {
            "account_id?:string,open_id?:string,app_token:string,name:string,default_view_name?:string,fields?:array"
        }
        "feishu.bitable.table.patch" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,name:string"
        }
        "feishu.bitable.table.batch_create" => {
            "account_id?:string,open_id?:string,app_token:string,tables:array"
        }
        "feishu.bitable.record.create" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,fields:object"
        }
        "feishu.bitable.record.update" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,record_id:string,fields:object"
        }
        "feishu.bitable.record.delete" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,record_id:string"
        }
        "feishu.bitable.record.batch_create" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,records:array"
        }
        "feishu.bitable.record.batch_update" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,records:array"
        }
        "feishu.bitable.record.batch_delete" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,records:array"
        }
        "feishu.bitable.field.create" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,field_name:string,type:integer,property?:object"
        }
        "feishu.bitable.field.list" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,view_id?:string,page_size?:integer,page_token?:string"
        }
        "feishu.bitable.field.update" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,field_id:string,field_name:string,type:integer,property?:object"
        }
        "feishu.bitable.field.delete" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,field_id:string"
        }
        "feishu.bitable.view.create" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,view_name:string,view_type?:string"
        }
        "feishu.bitable.view.get" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,view_id:string"
        }
        "feishu.bitable.view.list" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,page_size?:integer,page_token?:string"
        }
        "feishu.bitable.view.patch" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,view_id:string,view_name:string"
        }
        "feishu.bitable.record.search" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,view_id?:string,filter?:object,sort?:array,field_names?:string[],automatic_fields?:boolean,page_size?:integer,page_token?:string"
        }
        "feishu.calendar.freebusy" => {
            "account_id?:string,open_id?:string,time_min:string,time_max:string,user_id?:string,room_id?:string"
        }
        "feishu.calendar.list" => {
            "account_id?:string,open_id?:string,primary?:boolean,page_size?:integer,page_token?:string,sync_token?:string"
        }
        "feishu.calendar.primary.get" => "account_id?:string,open_id?:string,user_id_type?:string",
        "feishu.card.update" => {
            "account_id?:string,callback_token?:string,card?:object,markdown?:string,shared?:boolean,open_ids?:string[]"
        }
        "feishu.doc.append" => {
            "account_id?:string,open_id?:string,url:string,content?:string,content_path?:string,content_type?:string"
        }
        "feishu.doc.create" => {
            "account_id?:string,open_id?:string,title?:string,folder_token?:string,content?:string,content_path?:string,content_type?:string"
        }
        "feishu.doc.read" => "account_id?:string,open_id?:string,url:string,lang?:integer",
        "feishu.messages.get" => "account_id?:string,open_id?:string,message_id:string",
        "feishu.messages.history" => {
            "account_id?:string,open_id?:string,container_id?:string,container_id_type?:string,page_size?:integer,page_token?:string"
        }
        #[cfg(feature = "tool-file")]
        "feishu.messages.resource.get" => {
            "account_id?:string,open_id?:string,message_id?:string,file_key?:string,type?:string,save_as?:string"
        }
        "feishu.messages.reply" => {
            "account_id?:string,open_id?:string,message_id:string,text?:string,post?:object,image_key?:string,file_key?:string,card?:object,markdown?:string"
        }
        "feishu.messages.search" => {
            "account_id?:string,open_id?:string,query:string,page_size?:integer,page_token?:string"
        }
        "feishu.messages.send" => {
            "account_id?:string,open_id?:string,receive_id:string,receive_id_type?:string,text?:string,post?:object,image_key?:string,file_key?:string,card?:object,markdown?:string"
        }
        "feishu.whoami" => "account_id?:string,open_id?:string",
        "tool.search" => "query?:string,exact_tool_id?:string,limit?:integer",
        "tool.invoke" => "tool_id:string,lease:string,arguments:object",
        "read" => {
            "path?:string,offset?:integer,limit?:integer,max_bytes?:integer,query?:string,pattern?:string,root?:string,glob?:string,max_results?:integer,max_bytes_per_file?:integer,case_sensitive?:boolean,include_directories?:boolean"
        }
        "write" => {
            "path:string,content?:string,create_dirs?:boolean,overwrite?:boolean,edits?:array,old_string?:string,new_string?:string,replace_all?:boolean"
        }
        "exec" => "command?:string,script?:string,args?:string[],timeout_ms?:integer,cwd?:string",
        "web" => {
            "url?:string,mode?:string,max_bytes?:integer,query?:string,provider?:string,max_results?:integer"
        }
        "browser" => {
            "url?:string,max_bytes?:integer,session_id?:string,mode?:string,selector?:string,limit?:integer,link_id?:integer"
        }
        "memory" => "query?:string,max_results?:integer,path?:string,from?:integer,lines?:integer",
        "config.import" => {
            "input_path?:string,output_path?:string,mode?:string,source?:string,source_id?:string,primary_source_id?:string,safe_profile_merge?:boolean,apply_external_skills_plan?:boolean,force?:boolean"
        }
        "external_skills.fetch" => {
            "reference?:string,url?:string,approval_granted?:boolean,save_as?:string,max_bytes?:integer"
        }
        "external_skills.resolve" => "reference:string",
        "external_skills.search" => "query:string,limit:integer",
        "external_skills.recommend" => "query:string,limit:integer",
        "external_skills.source_search" => "query:string,max_results?:integer,sources?:string[]",
        "external_skills.inspect" => "skill_id:string",
        "external_skills.install" => {
            "path?:string,bundled_skill_id?:string,skill_id?:string,source_skill_id?:string,security_decision?:string,replace?:boolean"
        }
        "external_skills.invoke" => "skill_id:string",
        "external_skills.list" => "",
        "external_skills.policy" => {
            "action?:string,enabled?:boolean,allowed_domains?:string[],blocked_domains?:string[]"
        }
        "external_skills.remove" => "skill_id:string",
        "browser.companion.session.start" => "url:string",
        "browser.companion.navigate" => "session_id:string,url:string",
        "browser.companion.snapshot" => "session_id:string,mode?:string",
        "browser.companion.wait" => "session_id:string,condition?:string,timeout_ms?:integer",
        "browser.companion.session.stop" => "session_id:string",
        "browser.companion.click" => "session_id:string,selector:string",
        "browser.companion.type" => "session_id:string,selector:string,text:string",
        "http.request" => {
            "url:string,method?:string,headers?:object,body?:string,content_type?:string,max_bytes?:integer"
        }
        "file.read" => "path:string,offset?:integer,limit?:integer,max_bytes?:integer",
        "glob.search" => {
            "pattern:string,root?:string,max_results?:integer,include_directories?:boolean"
        }
        "content.search" => {
            "query:string,root?:string,glob?:string,max_results?:integer,max_bytes_per_file?:integer,case_sensitive?:boolean"
        }
        "memory_search" => "query:string,max_results?:integer",
        "memory_get" => "path:string,from?:integer,lines?:integer",
        "file.write" => "path:string,content:string,create_dirs?:boolean,overwrite?:boolean",
        "file.edit" => {
            "path:string,edits?:array,old_string?:string,new_string?:string,replace_all?:boolean"
        }
        "shell.exec" => "command:string,args?:string[],timeout_ms?:integer,cwd?:string",
        "bash.exec" => "command:string,cwd?:string,timeout_ms?:integer",
        "provider.switch" => "selector?:string",
        "delegate" | "delegate_async" => {
            "task:string,label?:string,profile?:string,isolation?:string,timeout_seconds?:integer"
        }
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" => "session_id:string",
        "session_continue" => "session_id:string,input:string,timeout_seconds?:integer",
        "sessions_list" => "limit?:integer,offset?:integer,state?:string",
        "sessions_send" => "session_id:string,text:string",
        "web.search" => "query:string,provider?:string,max_results?:integer",
        _ => "",
    }
}

pub(super) fn tool_search_hint(name: &str, fallback: &'static str) -> &'static str {
    if let Some(search_hint) = crate::tools::tool_surface::direct_tool_search_hint(name) {
        return search_hint;
    }

    match name {
        "tool.search" => {
            "discover a specialized tool when the visible direct tools do not fit, or refresh a known tool card"
        }
        "tool.invoke" => "run a discovered specialized tool with the lease returned by tool.search",
        "http.request" => {
            "send a bounded http request, inspect status and headers, fetch text or binary responses"
        }
        "file.read" => {
            "read a workspace file, inspect file contents, or page through a file window"
        }
        "glob.search" => {
            "find workspace files by glob pattern, list files in a directory, browse folder contents, search repo paths, match files under a root"
        }
        "content.search" => {
            "search workspace file contents, find text in repo files, grep text in the project"
        }
        "file.write" => {
            "write a workspace file, save file content, create or overwrite a repo file"
        }
        "file.edit" => {
            "edit a workspace file, patch file content, or apply exact replacement blocks in a repo file"
        }
        "shell.exec" => {
            "run a shell command, execute a terminal command, bash, zsh, powershell, cli"
        }
        "web.fetch" => "fetch a web page, download page text, inspect http content from a url",
        "web.search" => "search the web, look up web results, find information online",
        "memory_search" => {
            "search durable workspace memory, recall prior notes, query stored memory"
        }
        "memory_get" => "read a memory note by path, inspect saved durable memory content",
        "provider.switch" => "switch model provider, change runtime provider selection",
        _ => fallback,
    }
}

pub(super) fn tool_parameter_types(name: &str) -> &'static [(&'static str, &'static str)] {
    if let Some(parameter_types) = crate::tools::tool_surface::direct_tool_parameter_types(name) {
        return parameter_types;
    }

    match name {
        "feishu.bitable.app.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("name", "string"),
            ("folder_token", "string"),
        ],
        "feishu.bitable.app.get" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
        ],
        "feishu.bitable.app.list" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("folder_token", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.bitable.app.patch" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("name", "string"),
            ("is_advanced", "boolean"),
        ],
        "feishu.bitable.app.copy" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("name", "string"),
            ("folder_token", "string"),
        ],
        "feishu.bitable.list" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.bitable.table.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("name", "string"),
            ("default_view_name", "string"),
            ("fields", "array"),
        ],
        "feishu.bitable.table.patch" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("name", "string"),
        ],
        "feishu.bitable.table.batch_create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("tables", "array"),
        ],
        "feishu.bitable.record.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("fields", "object"),
        ],
        "feishu.bitable.record.update" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("record_id", "string"),
            ("fields", "object"),
        ],
        "feishu.bitable.record.delete" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("record_id", "string"),
        ],
        "feishu.bitable.record.batch_create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("records", "array"),
        ],
        "feishu.bitable.record.batch_update" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("records", "array"),
        ],
        "feishu.bitable.record.batch_delete" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("records", "array"),
        ],
        "feishu.bitable.field.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("field_name", "string"),
            ("type", "integer"),
            ("property", "object"),
        ],
        "feishu.bitable.field.list" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("view_id", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.bitable.field.update" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("field_id", "string"),
            ("field_name", "string"),
            ("type", "integer"),
            ("property", "object"),
        ],
        "feishu.bitable.field.delete" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("field_id", "string"),
        ],
        "feishu.bitable.view.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("view_name", "string"),
            ("view_type", "string"),
        ],
        "feishu.bitable.view.get" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("view_id", "string"),
        ],
        "feishu.bitable.view.list" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.bitable.view.patch" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("view_id", "string"),
            ("view_name", "string"),
        ],
        "feishu.bitable.record.search" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("view_id", "string"),
            ("filter", "object"),
            ("sort", "array"),
            ("field_names", "array"),
            ("automatic_fields", "boolean"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.calendar.freebusy" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("time_min", "string"),
            ("time_max", "string"),
            ("user_id", "string"),
            ("room_id", "string"),
        ],
        "feishu.calendar.list" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("primary", "boolean"),
            ("page_size", "integer"),
            ("page_token", "string"),
            ("sync_token", "string"),
        ],
        "feishu.calendar.primary.get" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("user_id_type", "string"),
        ],
        "feishu.card.update" => &[
            ("account_id", "string"),
            ("callback_token", "string"),
            ("card", "object"),
            ("markdown", "string"),
            ("shared", "boolean"),
            ("open_ids", "array"),
        ],
        "feishu.doc.append" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("url", "string"),
            ("content", "string"),
            ("content_path", "string"),
            ("content_type", "string"),
        ],
        "feishu.doc.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("title", "string"),
            ("folder_token", "string"),
            ("content", "string"),
            ("content_path", "string"),
            ("content_type", "string"),
        ],
        "feishu.doc.read" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("url", "string"),
            ("lang", "integer"),
        ],
        "feishu.messages.get" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("message_id", "string"),
        ],
        "feishu.messages.history" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("container_id", "string"),
            ("container_id_type", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        #[cfg(feature = "tool-file")]
        "feishu.messages.resource.get" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("message_id", "string"),
            ("file_key", "string"),
            ("type", "string"),
            ("save_as", "string"),
        ],
        "feishu.messages.reply" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("message_id", "string"),
            ("text", "string"),
            ("post", "object"),
            ("image_key", "string"),
            ("file_key", "string"),
            ("card", "object"),
            ("markdown", "string"),
        ],
        "feishu.messages.search" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("query", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.messages.send" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("receive_id", "string"),
            ("receive_id_type", "string"),
            ("text", "string"),
            ("post", "object"),
            ("image_key", "string"),
            ("file_key", "string"),
            ("card", "object"),
            ("markdown", "string"),
        ],
        "feishu.whoami" => &[("account_id", "string"), ("open_id", "string")],
        "tool.search" => &[
            ("query", "string"),
            ("exact_tool_id", "string"),
            ("limit", "integer"),
        ],
        "tool.invoke" => &[
            ("tool_id", "string"),
            ("lease", "string"),
            ("arguments", "object"),
        ],
        "config.import" => &[
            ("input_path", "string"),
            ("output_path", "string"),
            ("mode", "string"),
            ("source", "string"),
            ("source_id", "string"),
            ("primary_source_id", "string"),
            ("safe_profile_merge", "boolean"),
            ("apply_external_skills_plan", "boolean"),
            ("force", "boolean"),
        ],
        "external_skills.fetch" => &[
            ("reference", "string"),
            ("url", "string"),
            ("approval_granted", "boolean"),
            ("save_as", "string"),
            ("max_bytes", "integer"),
        ],
        "external_skills.resolve" => &[("reference", "string")],
        "external_skills.search" => &[("query", "string"), ("limit", "integer")],
        "external_skills.recommend" => &[("query", "string"), ("limit", "integer")],
        "external_skills.source_search" => &[
            ("query", "string"),
            ("max_results", "integer"),
            ("sources", "array"),
        ],
        "external_skills.inspect" | "external_skills.invoke" | "external_skills.remove" => {
            &[("skill_id", "string")]
        }
        "external_skills.install" => &[
            ("path", "string"),
            ("bundled_skill_id", "string"),
            ("skill_id", "string"),
            ("source_skill_id", "string"),
            ("security_decision", "string"),
            ("replace", "boolean"),
        ],
        "external_skills.list" => &[],
        "browser.companion.session.start" => &[("url", "string")],
        "browser.companion.navigate" => &[("session_id", "string"), ("url", "string")],
        "browser.companion.snapshot" => &[("session_id", "string"), ("mode", "string")],
        "browser.companion.wait" => &[
            ("session_id", "string"),
            ("condition", "string"),
            ("timeout_ms", "integer"),
        ],
        "browser.companion.session.stop" => &[("session_id", "string")],
        "browser.companion.click" => &[("session_id", "string"), ("selector", "string")],
        "browser.companion.type" => &[
            ("session_id", "string"),
            ("selector", "string"),
            ("text", "string"),
        ],
        "http.request" => &[
            ("url", "string"),
            ("method", "string"),
            ("headers", "object"),
            ("body", "string"),
            ("content_type", "string"),
            ("max_bytes", "integer"),
        ],
        "external_skills.policy" => &[
            ("action", "string"),
            ("enabled", "boolean"),
            ("allowed_domains", "array"),
            ("blocked_domains", "array"),
        ],
        "file.read" => &[
            ("path", "string"),
            ("offset", "integer"),
            ("limit", "integer"),
            ("max_bytes", "integer"),
        ],
        "glob.search" => &[
            ("pattern", "string"),
            ("root", "string"),
            ("max_results", "integer"),
            ("include_directories", "boolean"),
        ],
        "content.search" => &[
            ("query", "string"),
            ("root", "string"),
            ("glob", "string"),
            ("max_results", "integer"),
            ("max_bytes_per_file", "integer"),
            ("case_sensitive", "boolean"),
        ],
        "memory_search" => &[("query", "string"), ("max_results", "integer")],
        "memory_get" => &[
            ("path", "string"),
            ("from", "integer"),
            ("lines", "integer"),
        ],
        "file.write" => &[
            ("path", "string"),
            ("content", "string"),
            ("create_dirs", "boolean"),
            ("overwrite", "boolean"),
        ],
        "file.edit" => &[
            ("path", "string"),
            ("edits", "array"),
            ("old_string", "string"),
            ("new_string", "string"),
            ("replace_all", "boolean"),
        ],
        "shell.exec" => &[
            ("command", "string"),
            ("args", "array"),
            ("timeout_ms", "integer"),
            ("cwd", "string"),
        ],
        "bash.exec" => &[
            ("command", "string"),
            ("cwd", "string"),
            ("timeout_ms", "integer"),
        ],
        "provider.switch" => &[("selector", "string")],
        "delegate" | "delegate_async" => &[
            ("task", "string"),
            ("label", "string"),
            ("profile", "string"),
            ("isolation", "string"),
            ("timeout_seconds", "integer"),
        ],
        "session_continue" => &[
            ("session_id", "string"),
            ("input", "string"),
            ("timeout_seconds", "integer"),
        ],
        "session_tool_policy_status" | "session_tool_policy_clear" => &[("session_id", "string")],
        "session_tool_policy_set" => &[
            ("session_id", "string"),
            ("tool_ids", "array"),
            ("runtime_narrowing", "object"),
        ],
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" => &[("session_id", "string")],
        "sessions_list" => &[
            ("limit", "integer"),
            ("offset", "integer"),
            ("state", "string"),
        ],
        "session_search" => &[
            ("query", "string"),
            ("session_id", "string"),
            ("max_results", "integer"),
            ("include_archived", "boolean"),
            ("include_turns", "boolean"),
            ("include_events", "boolean"),
        ],
        "sessions_send" => &[("session_id", "string"), ("text", "string")],
        "web.search" => &[
            ("query", "string"),
            ("provider", "string"),
            ("max_results", "integer"),
        ],
        _ => &[],
    }
}

pub(super) fn tool_required_fields(name: &str) -> &'static [&'static str] {
    if let Some(required_fields) = crate::tools::tool_surface::direct_tool_required_fields(name) {
        return required_fields;
    }

    match name {
        "feishu.bitable.app.create" => &["name"],
        "feishu.bitable.app.get" => &["app_token"],
        "feishu.bitable.app.list" => &[],
        "feishu.bitable.app.patch" => &["app_token"],
        "feishu.bitable.app.copy" => &["app_token", "name"],
        "feishu.bitable.list" => &["app_token"],
        "feishu.bitable.table.create" => &["app_token", "name"],
        "feishu.bitable.table.patch" => &["app_token", "table_id", "name"],
        "feishu.bitable.table.batch_create" => &["app_token", "tables"],
        "feishu.bitable.record.create" => &["app_token", "table_id", "fields"],
        "feishu.bitable.record.update" => &["app_token", "table_id", "record_id", "fields"],
        "feishu.bitable.record.delete" => &["app_token", "table_id", "record_id"],
        "feishu.bitable.record.batch_create"
        | "feishu.bitable.record.batch_update"
        | "feishu.bitable.record.batch_delete" => &["app_token", "table_id", "records"],
        "feishu.bitable.field.create" => &["app_token", "table_id", "field_name", "type"],
        "feishu.bitable.field.list" => &["app_token", "table_id"],
        "feishu.bitable.field.update" => {
            &["app_token", "table_id", "field_id", "field_name", "type"]
        }
        "feishu.bitable.field.delete" => &["app_token", "table_id", "field_id"],
        "feishu.bitable.view.create" => &["app_token", "table_id", "view_name"],
        "feishu.bitable.view.get" => &["app_token", "table_id", "view_id"],
        "feishu.bitable.view.list" => &["app_token", "table_id"],
        "feishu.bitable.view.patch" => &["app_token", "table_id", "view_id", "view_name"],
        "feishu.bitable.record.search" => &["app_token", "table_id"],
        "feishu.calendar.freebusy" => &["time_min", "time_max"],
        "feishu.calendar.primary.get" => &[],
        "feishu.doc.append" | "feishu.doc.read" => &["url"],
        "feishu.messages.get" => &["message_id"],
        "feishu.messages.reply" => &["message_id"],
        "feishu.messages.search" => &["query"],
        "feishu.messages.send" => &["receive_id"],
        "tool.search" => &[],
        "tool.invoke" => &["tool_id", "lease", "arguments"],
        "external_skills.fetch" => &[],
        "external_skills.resolve" => &["reference"],
        "external_skills.search" => &["query", "limit"],
        "external_skills.recommend" => &["query", "limit"],
        "external_skills.source_search" => &["query"],
        "external_skills.inspect" | "external_skills.invoke" | "external_skills.remove" => {
            &["skill_id"]
        }
        // Grouped requirements are the source of truth for this tool's anyOf shape.
        "external_skills.install" => &[],
        "browser.companion.session.start" => &["url"],
        "browser.companion.navigate" => &["session_id", "url"],
        "browser.companion.snapshot"
        | "browser.companion.wait"
        | "browser.companion.session.stop" => &["session_id"],
        "browser.companion.click" => &["session_id", "selector"],
        "browser.companion.type" => &["session_id", "selector", "text"],
        "http.request" => &["url"],
        "file.read" => &["path"],
        "glob.search" => &["pattern"],
        "content.search" => &["query"],
        "memory_search" => &["query"],
        "memory_get" => &["path"],
        "file.write" => &["path", "content"],
        "file.edit" => &["path"],
        "shell.exec" => &["command"],
        "bash.exec" => &["command"],
        "delegate" | "delegate_async" => &["task"],
        "session_tool_policy_status" | "session_tool_policy_clear" => &[],
        "session_tool_policy_set" => &[],
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" => &["session_id"],
        "session_continue" => &["session_id", "input"],
        "sessions_send" => &["session_id", "text"],
        "web.search" => &["query"],
        _ => &[],
    }
}

pub(super) fn tool_tags(name: &str) -> &'static [&'static str] {
    if let Some(tags) = crate::tools::tool_surface::direct_tool_tags(name) {
        return tags;
    }

    match name {
        "feishu.bitable.app.get" | "feishu.bitable.app.list" => {
            &["feishu", "bitable", "app", "read"]
        }
        "feishu.bitable.app.create" | "feishu.bitable.app.patch" | "feishu.bitable.app.copy" => {
            &["feishu", "bitable", "app", "write"]
        }
        "feishu.bitable.list" | "feishu.bitable.record.search" => &["feishu", "bitable", "read"],
        "feishu.bitable.table.create"
        | "feishu.bitable.table.patch"
        | "feishu.bitable.table.batch_create" => &["feishu", "bitable", "table", "write"],
        "feishu.bitable.record.create"
        | "feishu.bitable.record.update"
        | "feishu.bitable.record.delete"
        | "feishu.bitable.record.batch_create"
        | "feishu.bitable.record.batch_update"
        | "feishu.bitable.record.batch_delete" => &["feishu", "bitable", "write"],
        "feishu.bitable.field.list" => &["feishu", "bitable", "field", "read"],
        "feishu.bitable.field.create"
        | "feishu.bitable.field.update"
        | "feishu.bitable.field.delete" => &["feishu", "bitable", "field", "write"],
        "feishu.bitable.view.get" | "feishu.bitable.view.list" => {
            &["feishu", "bitable", "view", "read"]
        }
        "feishu.bitable.view.create" | "feishu.bitable.view.patch" => {
            &["feishu", "bitable", "view", "write"]
        }
        "feishu.calendar.freebusy" | "feishu.calendar.list" | "feishu.calendar.primary.get" => {
            &["feishu", "calendar", "read"]
        }
        "feishu.card.update" => &["feishu", "card", "update", "callback"],
        "feishu.doc.read" => &["feishu", "docs", "read"],
        "feishu.doc.create" | "feishu.doc.append" => &["feishu", "docs", "write"],
        "feishu.messages.get" | "feishu.messages.history" | "feishu.messages.search" => {
            &["feishu", "messages", "read"]
        }
        #[cfg(feature = "tool-file")]
        "feishu.messages.resource.get" => &["feishu", "messages", "resource", "file"],
        "feishu.messages.send" | "feishu.messages.reply" => &["feishu", "messages", "write"],
        "feishu.whoami" => &["feishu", "identity", "read"],
        "tool.search" => &["core", "discover", "search"],
        "tool.invoke" => &["core", "dispatch", "invoke"],
        "config.import" => &["config", "import", "migration", "workspace", "legacy"],
        "external_skills.fetch" => &["skills", "download", "external", "fetch"],
        "external_skills.resolve" => &["skills", "resolve", "normalize", "external"],
        "external_skills.search" => &["skills", "search", "inventory", "discover"],
        "external_skills.recommend" => &["skills", "recommend", "inventory", "discover"],
        "external_skills.source_search" => &["skills", "search", "discover", "external"],
        "external_skills.inspect" => &["skills", "inspect", "metadata"],
        "external_skills.install" => &["skills", "install", "package"],
        "external_skills.invoke" => &["skills", "invoke", "instructions"],
        "external_skills.list" => &["skills", "list", "discover"],
        "external_skills.policy" => &["skills", "policy", "security"],
        "external_skills.remove" => &["skills", "remove", "uninstall"],
        "browser.companion.session.start"
        | "browser.companion.navigate"
        | "browser.companion.snapshot"
        | "browser.companion.wait"
        | "browser.companion.session.stop" => &["browser", "companion", "session", "read"],
        "browser.companion.click" | "browser.companion.type" => {
            &["browser", "companion", "write", "approval"]
        }
        "http.request" => &["http", "request", "web", "network", "external"],
        "file.read" => &["file", "read", "filesystem", "repo"],
        "glob.search" => &[
            "file",
            "search",
            "glob",
            "filesystem",
            "repo",
            "directory",
            "folder",
            "list",
            "browse",
        ],
        "content.search" => &["file", "search", "content", "filesystem", "repo"],
        "memory_search" => &["memory", "search", "recall", "durable", "workspace"],
        "memory_get" => &["memory", "read", "recall", "durable", "workspace"],
        "file.write" => &["file", "write", "filesystem"],
        "file.edit" => &["file", "edit", "filesystem", "exact", "replace"],
        "shell.exec" => &["shell", "command", "process", "exec"],
        "bash.exec" => &["bash", "command", "process", "exec"],
        "provider.switch" => &["provider", "switch", "model", "runtime"],
        "delegate" | "delegate_async" => &["session", "delegate", "child"],
        "session_tool_policy_status" | "session_tool_policy_set" | "session_tool_policy_clear" => {
            &["session", "policy", "tools", "security"]
        }
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" | "sessions_list" => {
            &["session", "history", "runtime"]
        }
        "session_continue" => &["session", "continue", "delegate", "child"],
        "sessions_send" => &["session", "message", "channel"],
        "web.search" => &["web", "search", "discover", "external"],
        _ => &[],
    }
}
