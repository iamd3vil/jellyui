{#- Notes files are rendered as templates: wrap literal {{ or {% in {% raw %}. -#}
{% include "release-notes/" ~ meta.tag ~ ".md" ignore missing %}
{%- for commit in commits %}
{%- if loop.first %}

## Commits
{% endif %}
* {{ commit.subject }} in {{ commit.hash }}
{%- endfor %}

**Full Changelog**: https://github.com/iamd3vil/jellyui/compare/{{ meta.previous_tag }}...{{ meta.tag }}
