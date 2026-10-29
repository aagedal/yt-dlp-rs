function setPosterPosition(img, player) {
    var img_w = img.width()
    var img_h = img.height();
    var img_new_w;
    var player_w = player.width();
    var player_h = player.height();
    var position = {};
    if (player_w / player_h > img_w / img_h) {
        img_new_w = img_w / img_h * player_h;
        position = {
            height: player_h + 'px',
            width: Math.round(img_new_w) + 'px',
            top: '0px',
            left: Math.round((player_w - img_new_w) / 2) + 'px'
        };
    } else {
        img_new_h = player_w * img_g / img_w;
        position = {
            height: Math.round(img_new_h) + 'px',
            width: player_w + 'px',
            left: '0px',
            top: Math.round((player_h - img_new_h) / 2) + 'px'
        };
    }
    img.css(position);
}
function load_chartbeat(cb_uid, cb_domain, cb_title, use_video) {
    window._sf_startpt = (new Date()).getTime();
    window._sf_async_config = { uid: cb_uid, domain: (cb_domain ? cb_domain : document.location.host), useCanonical: true, title: cb_title, useSubDomains: !!cb_domain};
    window._sf_endpt = (new Date()).getTime();
    var e = document.createElement('script');
    e.setAttribute('language', 'javascript');
    e.setAttribute('type', 'text/javascript');
    e.setAttribute('src', '//static.chartbeat.com/js/chartbeat_video.js');
    document.body.appendChild(e);
}
function checkScriptSource(script_src, media_url) {
    if (!script_src || script_src == '') return false;
    var schema_i = 2;
    var domain_i = 4;
    var query_i = 5;
    if (script_src == media_url) return true;
    var regexp = RegExp("^(([^:/?#]+):)?(//([^/?#]*))?([^?#]*)(\\?([^#]*))?(#(.*))?");
    var media_url_params = media_url.match(regexp);
    var script_src_params = script_src.match(regexp);
    if (script_src_params[domain_i] == media_url_params[domain_i] && media_url_params[query_i] == script_src_params[query_i]) {
        if (typeof script_src_params[schema_i] == 'undefined') {
            return true;
        } else {
            return script_src_params[schema_i] == media_url_params[schema_i];
        }
    }
}
function isJWPlayerLoaded(script_source) {
    var have_jw = false;
    for (var i = 0; i < document.scripts.length && !have_jw; i++) {
        if (
            document.scripts[i].src.indexOf('jwpsrv.com') > -1 ||
            document.scripts[i].src.indexOf('content.jwplatform.com') > -1 ||
            document.scripts[i].src.indexOf('jwpcdn.com') > -1 ||
            document.scripts[i].src.indexOf('jwplayer.js') > -1
        ) {
            have_jw = true;
        }
    }
    if (!have_jw) {
        var jw_script = document.createElement('script');
        jw_script.setAttribute('src', script_source);
        document.body.appendChild(jw_script);
    }
    return (typeof(jwplayer) != 'undefined');
}
function isFlowPlayerLoaded(skin) {
    var script, link;
    var have_flow = 0;
    var have_jquery = false;
    for (var i = 0; i < document.scripts.length; i++) {
        if (document.scripts[i].src.indexOf('flowplayer.min.js') > -1 || document.scripts[i].src.indexOf('flowplayer.hlsjs.min.js') > -1)
            have_flow += 1;
        if (document.scripts[i].src.indexOf('jquery') > -1)
            have_jquery = true;
    }
    if (!have_jquery) {
        script = document.createElement('script');
        script.setAttribute('src', '//code.jquery.com/jquery-1.12.4.min.js');
        document.body.appendChild(script);
    }
    have_flow = (have_flow == 2);
    if (!have_flow && typeof(jQuery) != 'undefined') {
        script = document.createElement('script');
        script.setAttribute('src', '//releases.flowplayer.org/7.0.2/flowplayer.min.js');
        document.body.appendChild(script);
        script = document.createElement('script');
        script.setAttribute('src', '//releases.flowplayer.org/hlsjs/flowplayer.hlsjs.min.js');
        document.body.appendChild(script);
        link = document.createElement('link');
        link.setAttribute('rel', 'stylesheet');
        link.setAttribute('href', skin || '//releases.flowplayer.org/7.0.2/skin/skin.css');
        document.body.appendChild(link);
    }
    return (typeof(flowplayer) != 'undefined');
}
function isGALoaded() {
    if (typeof window.ga_start_time == 'undefined') window.ga_start_time = (new Date()).getTime();
    var have_ga = false;
    for (var i = 0; i < document.scripts.length && !have_ga; i++) {
        if (
            document.scripts[i].src.indexOf('google-analytics.com/ga.js') > -1 ||
            document.scripts[i].src.indexOf('doubleclick.net/dc.js') > -1
        ) {
            have_ga = true;
        }
    }
    if (!have_ga) {
        var ga = document.createElement('script');
        ga.type = 'text/javascript';
        ga.async = true;
        ga.src = ('https:' == document.location.protocol ? 'https://ssl' : 'http://www') + '.google-analytics.com/ga.js';
        var s = document.getElementsByTagName('script')[0];
        s.parentNode.insertBefore(ga, s);
    }
    return (typeof(_gat) != 'undefined') || ((new Date()).getTime() >= window.ga_start_time + 3000);
}
function getJWPlayerEventLabel(item) {
    if (typeof item.ga_label != 'undefined') {
        label = item.ga_label;
    } else if (typeof item.title != 'undefined') {
        label = item.title;
        if (item.title != item.mediaid) {
            label += ' - ' + item.mediaid;
        }
    } else {
        label = item.mediaid;
    }
    return label;
}
function isVidlyPlayerLoaded(player_js, player_css) {
    var have_vjs = false;
    for (var i = 0; i < document.scripts.length && !have_vjs; i++) {
        if (document.scripts[i].src.indexOf('vjs2.js') > -1) {
            have_vjs = true;
        }
    }
    if (!have_vjs) {
        window.version = 6;
        window.name = "main";
        window.usrc = document.location.href;
        var vjs_script = document.createElement('script');
        vjs_script.setAttribute('src', player_js);
        vjs_script.setAttribute('language', 'javascript');
        document.body.appendChild(vjs_script);
        var vjs_css = document.createElement('link');
        vjs_css.setAttribute('rel', 'stylesheet');
        vjs_css.setAttribute('href', player_css);
        document.body.appendChild(vjs_css);
    }
    return typeof vjs != 'undefined';
}
function initVidly(player_id, inner_params, vidlyconfig) {
    var track, tracker, i, ready = true;
    
    if(typeof flashObj == 'undefined' && vidlyconfig.flash) {
        window.flashObj = false;
        window.version = 6;
        window.name = "main";
        window.usrc = document.location.href;

        var flash_vbs = document.createElement("script");
        flash_vbs.setAttribute("language", "vbscript");
        flash_vbs.setAttribute("type", "text/vbscript");
        flash_vbs.src = vidlyconfig.player_js.replace("vjs2.js", "flash.vbs");
        document.body.appendChild(flash_vbs); 
    }
    
    
    ready = ready && isVidlyPlayerLoaded(vidlyconfig.player_js, vidlyconfig.player_css);
    if (vidlyconfig.ga) ready = ready && isGALoaded();
    if (!ready) {
        window.setTimeout(function() {
            initVidly(player_id, inner_params, vidlyconfig);
        }, 100);
        return;
    }
    
    tracker = 'pageTracker_' + Math.round(100000 * Math.random());
    window[tracker] = '';
    if (vidlyconfig.ga) {
        try {
            window[tracker] = _gat._createTracker(vidlyconfig.ga);
            window[tracker]._trackPageview();
        } catch (e) {
            console.log(e);
            window[tracker] = '';
        }
    }
    if (vidlyconfig.chartbeat)
        load_chartbeat(vidlyconfig.chartbeat.uid, vidlyconfig.chartbeat.domain, vidlyconfig.chartbeat.title, false);    
    
    
    window[player_id + "_obj"] = new vjs();
    var js_config = vidlyconfig;
    if (vidlyconfig.ga) js_config.ga = window[tracker];
    window[player_id + "_obj"].init(player_id, js_config);
}
function initFlow(player_id, inner_params, flowconfig) {
    var ready = true;
    ready = ready && isFlowPlayerLoaded(flowconfig.skin);
    if (!ready) {
        window.setTimeout(function() {
            initFlow(player_id, inner_params, flowconfig);
        }, 100);
        return;
    }
    flowplayer.conf = {
        embed: false,
        fullscreen: true
    };
    var source, track;
    var video = jQuery("<video></video>");
    // video.attr("poster", flowconfig.poster);
    for (var i in flowconfig.medias) {
        source = jQuery("<source></source>");
        source.attr('type', flowconfig.medias[i].type);
        source.attr('src', flowconfig.medias[i].src);
        video.append(source);
    }
    if (flowconfig.cc) {
        track = jQuery('<track></track>');
        track.attr('src', flowconfig.cc);
        video.append(track);
    }
    if (flowconfig.poster)
        jQuery("#" + player_id).addClass('is-splash').css('background-image', 'url(' + flowconfig.poster + ')');
    jQuery("#" + player_id).append(video);
    jQuery("#" + player_id).flowplayer();
}
function initJW(player_id, inner_params, jwconfig) {
    var tracker, ready = true;
    ready = ready && isJWPlayerLoaded(jwconfig.player_url);
    if (jwconfig.ga) ready = ready && isGALoaded();
    if (!ready) {
        window.setTimeout(function() {
            initJW(player_id, inner_params, jwconfig);
        }, 100);
        return;
    }
    tracker = 'pageTracker_' + Math.round(100000 * Math.random());
    window[tracker] = '';
    if (jwconfig.ga) {
        try {
            window[tracker] = _gat._createTracker(jwconfig.ga);
            window[tracker]._trackPageview();
        } catch (e) {
            console.log(e);
            window[tracker] = '';
        }
    }
	jwconfig.ga = null;
    if (jwconfig.chartbeat)
        load_chartbeat(jwconfig.chartbeat.uid, jwconfig.chartbeat.domain, jwconfig.chartbeat.title, true);
    if (!jwconfig.key && jwconfig.player_key)
        jwconfig.key = jwconfig.player_key;
    if (jwconfig.key) jwplayer.key = jwconfig.key;
    var jw_video_aspectratio = '16:9';
    var jw_video_width = '100%';
    var jw_video_height = '';
    var ref = document.getElementById(player_id)
    var parent = ref.parentNode
    if (typeof(parent) != 'undefined') {
        if (parent.getAttribute('data-player-aspectratio')) {
            jw_video_aspectratio = parent.getAttribute('data-player-aspectratio');
        }
        if (!parent.getAttribute('data-player-width') && !parent.getAttribute('data-player-height')) {
            jwconfig.width = jw_video_width;
            jwconfig.aspectratio = jw_video_aspectratio;
        } else if (parent.getAttribute('data-player-width') && parent.getAttribute('data-player-height')) {
            jwconfig.width = parent.getAttribute('data-player-width')
            jwconfig.height = parent.getAttribute('data-player-height')
        } else if (parent.getAttribute('data-player-width')) {
            jwconfig.width = parent.getAttribute('data-player-width')
            jwconfig.aspectratio = jw_video_aspectratio
        } else if (parent.getAttribute('data-player-height')) {
            jwconfig.height = parent.getAttribute('data-player-height')
            jwconfig.aspectratio = jw_video_aspectratio
        }
    } else {
        jwconfig.width = jw_video_width;
        jwconfig.aspectratio = jw_video_aspectratio;
    }
    // IE11, IE10, IE9, IE8 fix
    var _nua = ''
    var _napp = ''
    if (typeof navigator != 'undefined' && typeof navigator.userAgent != 'undefined')
        _nua = navigator.userAgent;
    if (typeof navigator != 'undefined' && typeof navigator.appVersion != 'undefined')
        _napp = navigator.appVersion;
    if (_nua.indexOf("Trident/7") != -1 && _nua.indexOf("rv:11") != -1) {
        jwconfig.primary = 'flash';
    } else if (_napp.indexOf("MSIE 10.") != -1 || _napp.indexOf("MSIE 9.") != -1 || _napp.indexOf("MSIE 8.") != -1) {
        jwconfig.primary = 'flash';
    }
    for (var i in inner_params) {
        jwconfig[i] = inner_params[i];
    }
    var jwstates = {}, jwprogress = {};
    jwplayer(player_id).setup(jwconfig).onReady(function() {
        if (document.getElementById(player_id).offsetWidth > 640) {
            var playlist = this.getPlaylist();
            for (var i in playlist) {
                if (playlist[i].image_hd)
                    playlist[i].image = playlist[i].image_hd;
            }
            this.load(playlist);
        }
        if(jwconfig.chartbeat) {
            window._cbv = window._cbv || []; 
            window._cbv.push(jwplayer);
        }
    }).onPlay(function(i) {
        if (typeof jwstates[this.getPlaylistItem().mediaid] == 'undefined') {
            jwstates[this.getPlaylistItem().mediaid] = 'UNDEF';
        }
        if (i.oldstate.toLowerCase() == 'buffering' && jwstates[this.getPlaylistItem().mediaid].toLowerCase() != 'playing') {
            jwstates[this.getPlaylistItem().mediaid] = "PLAYING";
            if (window[tracker]) {
                try {
                    window[tracker]._trackEvent('Vid.ly video', 'Play', getJWPlayerEventLabel(this.getPlaylistItem()));
                } catch (e) {
                }
            }
            if (jwconfig.track_src) {
                var track = document.createElement('img');
                track.src = jwconfig.track_src;
            }
        } else if(i.oldstate.toLowerCase() == 'paused') {
			if (window[tracker]) {
                try {
                    window[tracker]._trackEvent('Vid.ly video', 'Play', getJWPlayerEventLabel(this.getPlaylistItem()));
                } catch (e) {
                }
            }
		}
    }).onComplete(function() {
        if (jwstates[this.getPlaylistItem().mediaid] != 'DONE') {
            jwstates[this.getPlaylistItem().mediaid] = "DONE";
        }
    }).onPause(function(i) {
        if (i.oldstate == 'PLAYING' && jwstates[this.getPlaylistItem().mediaid] != 'PAUSED') {
            jwstates[this.getPlaylistItem().mediaid] = "PAUSED";
			if (window[tracker]) {
                try {
                    window[tracker]._trackEvent('Vid.ly video', 'Pause', getJWPlayerEventLabel(this.getPlaylistItem()));
                } catch (e) {
                }
            }
        }
    }).onTime(function(e) {
        if (typeof jwprogress[this.getPlaylistItem().mediaid] == 'undefined') {
            jwprogress[this.getPlaylistItem().mediaid] = 0;
        }
        var new_progress = parseFloat(e.position) / parseFloat(e.duration) * 100;
        var target_progress = Math.round(new_progress);
        if (target_progress > 100) return;
        if (jwprogress[this.getPlaylistItem().mediaid] > 0 && target_progress % 10 == 0) {
            if (window[tracker] && target_progress > jwprogress[this.getPlaylistItem().mediaid] && target_progress <= new_progress) {
                try {
                    window[tracker]._trackEvent('Vid.ly video', 'Video progress ' + target_progress + '%', getJWPlayerEventLabel(this.getPlaylistItem()));
                } catch (e) {
                }
            }
        }
        jwprogress[this.getPlaylistItem().mediaid] = new_progress
    })
}
function initCallback(json) {
    var i, temp;
    if (!json.player) return;
    if (!json.self_src) return;
    var scripts = document.getElementsByTagName('script');
    var player_id = json.player + '_' + Math.round(100000 * Math.random());
    var div, params = {};
    for (i = 0; i < scripts.length; i++) {
        if (!scripts[i].getAttribute('src')) continue;
        if (scripts[i].getAttribute('src').toLowerCase().replace(/^https?:/, '').replace(/\?.*/, '') != json.self_src.toLowerCase().replace(/^https?:/, '').replace(/\?.*/, '')) continue;
        if (scripts[i].getAttribute('data-vidly-deployed') == '1') continue;
        div = document.createElement('div');
        div.className = json.player + ' vidlyembed';
        
        scripts[i].parentNode.insertBefore(div, scripts[i]);
        scripts[i].setAttribute('data-vidly-deployed', '1');
        if (scripts[i].getAttribute('data-player-autoplay')) {
            temp = scripts[i].getAttribute('data-player-autoplay');
            params.autostart = (temp === '1' || temp === 'true');
        }
        if (scripts[i].getAttribute('data-player-controls')) {
            temp = scripts[i].getAttribute('data-player-controls');
            params.controls = (temp === '1' || temp === 'true');
        }
        if (json.player === 'jwplayer6' || json.player === 'jwplayer7') {
            div.id = player_id;
            initJW(player_id, params, json.config);
        } else if (json.player === 'vidly' || json.player === 'vidlyplayer') {            
            var video_player_id = player_id + Math.round(100000 * Math.random());
            var video = document.createElement('video');
            video.oncontextmenu = function () { return false; };
            video.setAttribute('id', video_player_id);
            video.className = 'vidly-js vidly-skin';
            video.setAttribute('poster', json.config.poster.src);
            video.style.height = "100%";
            video.setAttribute('src', json.config.source);
            video.setAttribute('type', json.config.type);
            var track = document.createElement('source');
            track.setAttribute('src', json.config.source);
            track.setAttribute('type', json.config.type);
            video.appendChild(track);
            div.appendChild(video);
            initVidly(video_player_id, params, json.config);
        } else if (json.player === 'flow' || json.player === 'flowplayer') {
            div.id = player_id;
            initFlow(player_id, params, json.config);
        }
    }
    
}
initCallback({"player":"jwplayer7","config":{"displaytitle":false,"primary":"flash","logo":{"link":"","hide":true},"player_url":"https:\/\/d132d9vcg4o0oh.cloudfront.net\/vid.ly\/f123700-stable\/js\/jw7\/jwplayer.js","player_key":"tFeSrk1JLY3A1LosCSoXd6vQeGPhyhNemCg8hw","analytics":{"enabled":true},"playlist":[{"image":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/poster.jpg?t=17933042506ae3a6ba13552","sources":[{"file":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/hls.m3u8","label":"HLS","type":"hls"},{"file":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/webm_360.webm","label":"360p","type":"webm"},{"file":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/webm_480.webm","label":"480p","type":"webm"},{"file":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/webm_720.webm","label":"720p","type":"webm"},{"file":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/webm_1080.webm","label":"1080p","type":"webm"},{"file":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/mp4_360.mp4","label":"360p","type":"mp4"},{"file":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/mp4_480.mp4","label":"480p","type":"mp4"},{"file":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/mp4_720.mp4","label":"720p","type":"mp4"},{"file":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/mp4_1080.mp4","label":"1080p","type":"mp4"}],"analytics":{"enabled":false},"mediaid":"2i3o9j","image_hd":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/poster_hd.jpg?t=17933042506ae3a6ba14bca","ga_label":"2i3o9j","title":"2i3o9j","tracks":[{"file":"https:\/\/d3fenhwk93s16g.cloudfront.net\/2i3o9j\/webvtt_thumb.vtt","kind":"thumbnails"}]}],"track_src":"https:\/\/vid.ly\/2i3o9j\/track?993082104"},"time":{"init":0.03,"config":0.03},"self_src":"https:\/\/vid.ly\/2i3o9j\/embed"});