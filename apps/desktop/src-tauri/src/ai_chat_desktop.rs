use crate::{ai_accounts::{self,AiAccounts,Provider},ai_chat::{self,AiChat},notion_context::{self,NotionContextService},require_main,DesktopState};
use serde::Serialize;
use serde_json::{json,Value};
use std::path::PathBuf;
use std::sync::{Arc,atomic::AtomicBool};
use tauri::{AppHandle,Emitter,Manager,State,WebviewWindow};

pub struct AiDesktop { pub chat:Option<Arc<AiChat>>,pub accounts:Option<Arc<AiAccounts>>,pub notion:Option<Arc<NotionContextService>>,account_error:Option<String>,notion_error:Option<String> }
impl AiDesktop{
 fn chat(&self)->Result<Arc<AiChat>,String>{self.chat.clone().ok_or_else(||self.account_error.clone().unwrap_or("AI 대화를 준비하지 못했습니다.".into()))}
 fn accounts(&self)->Result<Arc<AiAccounts>,String>{self.accounts.clone().ok_or_else(||self.account_error.clone().unwrap_or("AI 연결을 준비하지 못했습니다.".into()))}
 fn notion(&self)->Result<Arc<NotionContextService>,String>{self.notion.clone().ok_or_else(||self.notion_error.clone().unwrap_or("Notion 연결을 준비하지 못했습니다.".into()))}
}
pub fn install(app:&AppHandle,root:PathBuf,resources:PathBuf,core:nero_core::CoreHandle){
 let accounts=AiAccounts::new(root.clone(),resources);let notion=NotionContextService::new(root.clone());
 let account_error=accounts.as_ref().err().cloned();let notion_error=notion.as_ref().err().map(ToString::to_string);
 let accounts=accounts.ok().map(Arc::new);let notion=notion.ok().map(Arc::new);
 let chat=accounts.as_ref().map(|accounts|Arc::new(AiChat::new(root,core,accounts.clone(),notion.clone())));
 app.manage(AiDesktop{chat,accounts,notion,account_error,notion_error});
}
fn account_view(accounts:&AiAccounts)->Result<Value,String>{
 let snapshot=accounts.snapshot()?;let mut values=vec![];
 for account in snapshot.accounts{
  let state=if account.can_invoke{"ready"}else if account.state=="connecting"{"connecting"}else{"signed_out"};
  values.push(json!({"id":account.id,"provider":account.provider,"state":state,"label":account.label,"configured":account.configured,"detail":account.reason}));
 }
 for provider in &snapshot.providers{if !values.iter().any(|account|account["provider"]==serde_json::to_value(provider.id).unwrap()){
  values.push(json!({"id":"","provider":provider.id,"state":if provider.configured{"signed_out"}else{"unavailable"},"label":provider.name,"configured":provider.configured,"detail":provider.reason}));
 }}
 Ok(json!({"accounts":values,"providers":snapshot.providers,"activeAccountId":snapshot.active_account_id}))
}
async fn open(url:String)->Result<(),String>{
 let url=reqwest::Url::parse(&url).map_err(|_|"로그인 주소를 확인하지 못했습니다.")?;
 if url.scheme()!="https" || !matches!(url.host_str(),Some("auth.openai.com"|"claude.ai"|"platform.claude.com"|"mcp.notion.com"|"www.notion.so"|"notion.so"|"api.notion.com")){return Err("로그인 주소를 확인하지 못했습니다.".into());}
 tauri::async_runtime::spawn_blocking(move||crate::external_open::open_url(&url).map_err(|_|"기본 브라우저를 열지 못했습니다.".to_string())).await.map_err(|_|"기본 브라우저를 열지 못했습니다.".to_string())?
}
#[tauri::command]pub fn ai_chat_accounts(window:WebviewWindow,state:State<'_,AiDesktop>)->Result<Value,String>{require_main(&window)?;account_view(&state.accounts()?)}
#[derive(Serialize)]#[serde(rename_all="camelCase")]pub struct Attempt{attempt_id:String}
#[tauri::command]pub async fn ai_chat_connect(window:WebviewWindow,state:State<'_,AiDesktop>,provider:Provider)->Result<Attempt,String>{
 require_main(&window)?;let accounts=state.accounts()?;let attempt=accounts.begin_sign_in(provider,None)?;
 if let Err(error)=open(attempt.authorization_url).await{let _=accounts.cancel_sign_in(&attempt.id);return Err(error);}
 Ok(Attempt{attempt_id:attempt.id})
}
#[tauri::command]pub async fn ai_chat_finish_connect(window:WebviewWindow,state:State<'_,AiDesktop>,attempt_id:String)->Result<Value,String>{require_main(&window)?;let accounts=state.accounts()?;accounts.complete_sign_in(&attempt_id,Arc::new(AtomicBool::new(false))).await?;account_view(&accounts)}
#[tauri::command]pub fn ai_chat_cancel_connect(window:WebviewWindow,state:State<'_,AiDesktop>,attempt_id:String)->Result<Value,String>{require_main(&window)?;let accounts=state.accounts()?;accounts.cancel_sign_in(&attempt_id)?;account_view(&accounts)}
#[tauri::command]pub async fn ai_chat_disconnect(window:WebviewWindow,state:State<'_,AiDesktop>,account_id:String)->Result<Value,String>{require_main(&window)?;if let Some(chat)=&state.chat{chat.cancel_account(&account_id);}let accounts=state.accounts()?;accounts.sign_out(&account_id).await?;account_view(&accounts)}
#[tauri::command]pub async fn ai_chat_models(window:WebviewWindow,state:State<'_,AiDesktop>,account_id:String)->Result<ai_chat::Models,String>{require_main(&window)?;state.chat()?.models(&account_id).await}
#[tauri::command]pub async fn ai_chat_read(window:WebviewWindow,state:State<'_,AiDesktop>,meeting_id:String)->Result<ai_chat::Conversation,String>{require_main(&window)?;let chat=state.chat()?;tauri::async_runtime::spawn_blocking(move||chat.read(&meeting_id)).await.map_err(|_|"대화를 불러오지 못했습니다.".to_string())?}
#[tauri::command]pub async fn ai_chat_send(window:WebviewWindow,app:AppHandle,state:State<'_,AiDesktop>,request_id:String,conversation_id:Option<String>,meeting_id:String,include_meeting:bool,account_id:String,model_id:String,message:String,source_ids:Vec<String>,template_id:Option<String>)->Result<ai_chat::Started,String>{
 require_main(&window)?;let chat=state.chat()?;let request=ai_chat::SendRequest{request_id,conversation_id,meeting_id,include_meeting,account_id,model_id,message,source_ids,template_id};
 tauri::async_runtime::spawn_blocking(move||chat.start(request,Arc::new(move|event|{let _=app.emit_to("main","nero://ai-chat",event);}))).await.map_err(|_|"대화를 시작하지 못했습니다.".to_string())?
}
#[tauri::command]pub fn ai_chat_cancel(window:WebviewWindow,state:State<'_,AiDesktop>,request_id:String)->Result<(),String>{require_main(&window)?;state.chat()?.cancel(&request_id)}
async fn apply_answer(app:AppHandle,state:State<'_,AiDesktop>,conversation_id:String,message_id:String,meeting_id:String,summary:bool)->Result<Value,String>{
 let chat=state.chat()?;let result=tauri::async_runtime::spawn_blocking(move||if summary{chat.save_summary(&conversation_id,&message_id,&meeting_id)}else{chat.append_note(&conversation_id,&message_id,&meeting_id)}).await.map_err(|_|"답변을 저장하지 못했습니다.".to_string())??;
 let core=app.state::<DesktopState>().core.clone();if let Ok(snapshot)=crate::on_core(core,|core|core.snapshot()).await{crate::publish(&app,&snapshot);}Ok(result)
}
#[tauri::command]pub async fn ai_chat_save_summary(window:WebviewWindow,app:AppHandle,state:State<'_,AiDesktop>,conversation_id:String,message_id:String,meeting_id:String)->Result<Value,String>{require_main(&window)?;apply_answer(app,state,conversation_id,message_id,meeting_id,true).await}
#[tauri::command]pub async fn ai_chat_append_note(window:WebviewWindow,app:AppHandle,state:State<'_,AiDesktop>,conversation_id:String,message_id:String,meeting_id:String)->Result<Value,String>{require_main(&window)?;apply_answer(app,state,conversation_id,message_id,meeting_id,false).await}
#[tauri::command]pub async fn ai_chat_open_source(window:WebviewWindow,url:String)->Result<(),String>{require_main(&window)?;let url=reqwest::Url::parse(&url).map_err(|_|"자료 주소를 확인해 주세요.")?;if url.scheme()!="https" || !matches!(url.host_str(),Some("notion.so"|"www.notion.so"|"app.notion.com")) && !url.host_str().is_some_and(|host|host.ends_with(".notion.site")){return Err("자료 주소를 확인해 주세요.".into());}tauri::async_runtime::spawn_blocking(move||crate::external_open::open_url(&url).map_err(|_|"자료를 열지 못했습니다.".to_string())).await.map_err(|_|"자료를 열지 못했습니다.".to_string())?}
#[tauri::command]pub fn notion_connection_snapshot(window:WebviewWindow,state:State<'_,AiDesktop>)->Result<notion_context::NotionConnectionSnapshot,String>{require_main(&window)?;Ok(state.notion()?.snapshot())}
#[tauri::command]pub async fn begin_notion_connect(window:WebviewWindow,state:State<'_,AiDesktop>)->Result<Value,String>{require_main(&window)?;let service=state.notion()?;let request=service.begin_connect().await.map_err(|e|e.to_string())?;if let Err(error)=open(request.authorization_url).await{service.cancel_connect(Some(&request.attempt_id));return Err(error);}Ok(json!({"attemptId":request.attempt_id}))}
#[tauri::command]pub async fn finish_notion_connect(window:WebviewWindow,state:State<'_,AiDesktop>,attempt_id:String)->Result<notion_context::NotionConnectionSnapshot,String>{require_main(&window)?;state.notion()?.finish_connect(&attempt_id).await.map_err(|e|e.to_string())}
#[tauri::command]pub fn cancel_notion_connect(window:WebviewWindow,state:State<'_,AiDesktop>,attempt_id:Option<String>)->Result<notion_context::NotionConnectionSnapshot,String>{require_main(&window)?;let service=state.notion()?;service.cancel_connect(attempt_id.as_deref());Ok(service.snapshot())}
#[tauri::command]pub async fn disconnect_notion_context(window:WebviewWindow,state:State<'_,AiDesktop>)->Result<notion_context::NotionConnectionSnapshot,String>{require_main(&window)?;state.notion()?.disconnect().await.map_err(|e|e.to_string())}
#[tauri::command]pub async fn fetch_notion_sources(window:WebviewWindow,state:State<'_,AiDesktop>,request_id:String,links:Vec<String>)->Result<notion_context::NotionContextBundle,String>{require_main(&window)?;state.notion()?.fetch_sources(request_id,links).await.map_err(|e|e.to_string())}
#[tauri::command]pub fn cancel_notion_fetch(window:WebviewWindow,state:State<'_,AiDesktop>,request_id:String)->Result<(),String>{require_main(&window)?;state.notion()?.cancel_fetch(&request_id);Ok(())}
#[tauri::command]pub fn read_notion_sources(window:WebviewWindow,state:State<'_,AiDesktop>,source_ids:Vec<String>)->Result<notion_context::NotionContextBundle,String>{require_main(&window)?;state.notion()?.read_sources(source_ids).map_err(|e|e.to_string())}
