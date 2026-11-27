<?php
declare(strict_types=1);
require $argv[1] . '/app.php';
$db->exec("INSERT INTO users(id,name,password) VALUES(1,'fixture','not-a-real-password')");
$stmt=$db->prepare('INSERT INTO settings(key,value) VALUES(?,?)');
foreach (['name'=>'Rust 契约站','bio'=>'公开测试夹具，不含用户数据','handle'=>'fixture','plugin:rss:enabled'=>'1','plugin:rss:in_feed'=>'0','plugin:dms:enabled'=>'0','douban_enabled'=>'0','weibo_enabled'=>'0','wallpaper_enabled'=>'0'] as $key=>$value) $stmt->execute([$key,$value]);
$post=$db->prepare('INSERT INTO posts(id,content,created_at,updated_at,pinned,images,source,source_key,meta,link) VALUES(?,?,?,?,?,?,?,?,?,?)');
for($id=1;$id<=65;$id++) {
 $date='2026-01-' . str_pad((string)(1+intdiv($id-1,3)),2,'0',STR_PAD_LEFT) . 'T12:00:00+08:00';
 $post->execute([$id,'动态 '.$id.' 中文搜索 '.($id%2?'猫':'狗'),$date,$date,$id===2?1:0,$id===1?'["up:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png"]':'[]',$id===65?'rss':'',$id===65?'fixture:65':'','', '']);
}
$article=$db->prepare('INSERT INTO articles(id,title,content,format,images,tags,draft,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)');
foreach ([
 [1,'第一篇','# 标题'."\n\n".'正文 & 中文'."\n".'第二行','md','测试,猫',0],
 [2,'同时间文章','<p>富文本 &amp; 实体</p><script>alert(1)</script>','html','测试',0],
 [3,'私人草稿','secret draft','md','私有',1],
 [4,'封面文章','![封面](/image?upload=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png)','md','图片',0],
 [5,'危险内容','<img src=x onerror=alert(1)><a href="javascript:alert(1)">链接</a>','html','',0]
] as [$id,$title,$body,$format,$tags,$draft]) $article->execute([$id,$title,$body,$format,'[]',$tags,$draft,'2026-02-01T12:00:00+08:00','2026-02-01T12:00:00+08:00']);
$db->exec("INSERT INTO comments(kind,target_id,author,content,created_at,status) VALUES('post',2,'fixture','approved','2026-01-01','approved'),('post',2,'fixture','hidden','2026-01-01','pending'),('article',1,'fixture','approved','2026-01-01','approved')");
$db->exec("INSERT INTO post_likes(post_id,token,created_at) VALUES(2,'aaaaaaaaaaaaaaaa','2026-01-01')");
if(!is_dir(DATA_DIR.'/uploads')) mkdir(DATA_DIR.'/uploads',0755,true);
copy(__DIR__.'/../fixtures/rss/pixel.png',DATA_DIR.'/uploads/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png');
echo "fixture ready\n";
